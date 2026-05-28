use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::room::ClientHandle;
use crate::wire::{WireMessage, MAX_CLIP_BYTES};
use crate::AppState;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub room: String,
    pub token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(expected) = state.auth.get(&q.room) else {
        warn!(room = %q.room, "bilinmeyen oda");
        return (StatusCode::UNAUTHORIZED, "bilinmeyen oda").into_response();
    };
    if expected != &q.token {
        warn!(room = %q.room, "geçersiz token");
        return (StatusCode::UNAUTHORIZED, "geçersiz token").into_response();
    }
    let room = q.room;
    ws.max_message_size(MAX_CLIP_BYTES + 4096)
        .max_frame_size(MAX_CLIP_BYTES + 4096)
        .on_upgrade(move |socket| handle_socket(socket, room, state))
}

async fn handle_socket(mut socket: WebSocket, room: String, state: Arc<AppState>) {
    let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::channel::<WireMessage>(32);

    let last = state.rooms.join(
        &room,
        ClientHandle {
            id: client_id,
            tx: tx.clone(),
        },
    );
    info!(%room, client_id, "istemci bağlandı");

    // Bağlantı anında last varsa kendi kuyruğumuza koy
    if let Some(last_msg) = last {
        let _ = tx.send(last_msg).await;
    }

    loop {
        tokio::select! {
            outbound = rx.recv() => {
                let Some(msg) = outbound else { break };
                let text = match serde_json::to_string(&msg) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "outbound serialize hatası");
                        continue;
                    }
                };
                if socket.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            inbound = socket.recv() => {
                let Some(msg) = inbound else { break };
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(%room, client_id, error = %e, "ws hatası");
                        break;
                    }
                };
                match msg {
                    Message::Text(t) => {
                        let s: &str = t.as_str();
                        if s.len() > MAX_CLIP_BYTES {
                            warn!(%room, client_id, len = s.len(), "mesaj boyutu aşıldı, drop");
                            continue;
                        }
                        let wm: WireMessage = match serde_json::from_str(s) {
                            Ok(m) => m,
                            Err(e) => {
                                warn!(%room, client_id, error = %e, "json parse hatası");
                                continue;
                            }
                        };
                        match wm {
                            WireMessage::Clip { ref text, .. } => {
                                if text.len() > MAX_CLIP_BYTES {
                                    warn!(%room, client_id, len = text.len(), "clip text aşıldı");
                                    continue;
                                }
                                state.rooms.broadcast(&room, client_id, wm);
                            }
                            WireMessage::Ping => {
                                let _ = tx.send(WireMessage::Pong).await;
                            }
                            WireMessage::Pong => { /* yok say */ }
                        }
                    }
                    Message::Binary(_) => {
                        warn!(%room, client_id, "binary mesaj atlandı");
                    }
                    Message::Ping(p) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => break,
                }
            }
        }
    }

    state.rooms.leave(&room, client_id);
    info!(%room, client_id, "istemci ayrıldı");
}
