use std::time::Duration;

use anyhow::{Context, Result};
use chrono::SecondsFormat;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub const MAX_CLIP_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireMessage {
    Clip {
        text: String,
        source_machine: String,
        timestamp: String,
    },
    Ping,
    Pong,
}

pub struct WsConfig {
    pub url: String,
    pub room_id: String,
    pub token: String,
    pub source_machine: String,
}

#[derive(Debug, Clone)]
pub enum WsCommand {
    SendClip(String),
    Reconnect,
    #[allow(dead_code)]
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    Disconnected,
    Reconnecting { in_ms: u64 },
    Clip {
        text: String,
        source_machine: String,
        timestamp: String,
    },
}

enum SessionEnd {
    Shutdown,
    Closed,
}

pub async fn run(
    cfg: WsConfig,
    mut commands: mpsc::Receiver<WsCommand>,
    events: mpsc::Sender<WsEvent>,
) {
    let mut backoff_ms: u64 = 0;
    loop {
        if backoff_ms > 0 {
            let _ = events.send(WsEvent::Reconnecting { in_ms: backoff_ms }).await;
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
        let outcome = connect_and_run(&cfg, &mut commands, &events).await;
        match outcome {
            Ok(SessionEnd::Shutdown) => {
                eprintln!("[clipsync/ws] shutdown");
                return;
            }
            Ok(SessionEnd::Closed) => {
                eprintln!("[clipsync/ws] bağlantı kapandı");
            }
            Err(e) => {
                eprintln!("[clipsync/ws] hata: {e:#}");
            }
        }
        let _ = events.send(WsEvent::Disconnected).await;
        backoff_ms = if backoff_ms == 0 { 1_000 } else { (backoff_ms * 2).min(30_000) };
    }
}

async fn connect_and_run(
    cfg: &WsConfig,
    commands: &mut mpsc::Receiver<WsCommand>,
    events: &mpsc::Sender<WsEvent>,
) -> Result<SessionEnd> {
    let mut url = Url::parse(&cfg.url).context("ServerUrl parse")?;
    url.query_pairs_mut()
        .append_pair("room", &cfg.room_id)
        .append_pair("token", &cfg.token);
    let url_str = url.to_string();

    let ws_cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(MAX_CLIP_BYTES + 4096),
        max_frame_size: Some(MAX_CLIP_BYTES + 4096),
        ..Default::default()
    };

    let (ws_stream, _resp) =
        tokio_tungstenite::connect_async_with_config(&url_str, Some(ws_cfg), false)
            .await
            .context("ws connect")?;
    let _ = events.send(WsEvent::Connected).await;
    eprintln!("[clipsync/ws] bağlandı: {}", cfg.url);

    let (mut sink, mut stream) = ws_stream.split();
    let mut ping_tick = tokio::time::interval(Duration::from_secs(30));
    ping_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // İlk tick anında ateşler, atla
    ping_tick.tick().await;

    loop {
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { return Ok(SessionEnd::Shutdown) };
                match cmd {
                    WsCommand::SendClip(text) => {
                        let msg = WireMessage::Clip {
                            text,
                            source_machine: cfg.source_machine.clone(),
                            timestamp: chrono::Utc::now()
                                .to_rfc3339_opts(SecondsFormat::Secs, true),
                        };
                        let json = serde_json::to_string(&msg)?;
                        if let Err(e) = sink.send(Message::Text(json)).await {
                            eprintln!("[clipsync/ws] send hatası: {e}");
                            return Ok(SessionEnd::Closed);
                        }
                    }
                    WsCommand::Reconnect => {
                        let _ = sink.close().await;
                        return Ok(SessionEnd::Closed);
                    }
                    WsCommand::Shutdown => {
                        let _ = sink.close().await;
                        return Ok(SessionEnd::Shutdown);
                    }
                }
            }
            msg = stream.next() => {
                let Some(msg) = msg else { return Ok(SessionEnd::Closed) };
                let msg = msg.context("ws recv")?;
                match msg {
                    Message::Text(t) => {
                        let s: &str = t.as_str();
                        let wm: WireMessage = match serde_json::from_str(s) {
                            Ok(m) => m,
                            Err(e) => {
                                eprintln!("[clipsync/ws] json parse hatası, yutuldu: {e}");
                                continue;
                            }
                        };
                        match wm {
                            WireMessage::Clip { text, source_machine, timestamp } => {
                                let _ = events.send(WsEvent::Clip { text, source_machine, timestamp }).await;
                            }
                            WireMessage::Pong => { /* ack */ }
                            WireMessage::Ping => {
                                let json = serde_json::to_string(&WireMessage::Pong)?;
                                let _ = sink.send(Message::Text(json)).await;
                            }
                        }
                    }
                    Message::Close(_) => return Ok(SessionEnd::Closed),
                    _ => {}
                }
            }
            _ = ping_tick.tick() => {
                let json = serde_json::to_string(&WireMessage::Ping)?;
                if let Err(e) = sink.send(Message::Text(json)).await {
                    eprintln!("[clipsync/ws] ping send hatası: {e}");
                    return Ok(SessionEnd::Closed);
                }
            }
        }
    }
}
