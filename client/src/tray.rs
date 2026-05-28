#![cfg(windows)]

use std::sync::OnceLock;
use std::thread;

use anyhow::{Context, Result};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::{EventLoopBuilderExtWindows, WindowExtWindows};
use tao::window::WindowBuilder;
use tokio::runtime::Builder as TokioRtBuilder;
use tokio::sync::mpsc;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder, TrayIconEvent,
};
use windows::core::HSTRING;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MSG,
};

use crate::clipboard::{self, WM_CLIPBOARDUPDATE};
use crate::config::AppConfig;
use crate::sync::{LocalAction, SyncEngine};
use crate::ws;

#[derive(Debug)]
enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    Clipboard(String),
    Ws(ws::WsEvent),
}

static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

pub fn run(cfg: AppConfig) -> Result<()> {
    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    builder.with_msg_hook(|raw| {
        let msg = unsafe { &*(raw as *const MSG) };
        if msg.message == WM_CLIPBOARDUPDATE {
            if let Some(text) = clipboard::read_text() {
                if let Some(proxy) = PROXY.get() {
                    let _ = proxy.send_event(UserEvent::Clipboard(text));
                }
            }
        }
        false
    });
    let event_loop = builder.build();
    let _ = PROXY.set(event_loop.create_proxy());

    let hidden = WindowBuilder::new()
        .with_visible(false)
        .with_decorations(false)
        .with_title("clipsync-listener")
        .build(&event_loop)
        .context("görünmez pencere oluşturulamadı")?;
    let hwnd = HWND(hidden.hwnd() as _);
    clipboard::register_listener(hwnd).context("pano dinleyici kaydedilemedi")?;

    // Açılışta panoyu oku ve baseline olarak ata (kendi içeriği boş yere yüklenmesin)
    let initial_clip = clipboard::read_text();
    let mut engine = SyncEngine::new(hostname(), initial_clip.clone());

    eprintln!(
        "[clipsync] config: server={} room={}",
        cfg.server_url, cfg.room_id
    );
    eprintln!(
        "[clipsync] pano dinleyici hazır (hwnd={:?}), açılış panosu: {} char",
        hwnd.0,
        initial_clip.as_deref().map(|s| s.chars().count()).unwrap_or(0)
    );

    // WS thread
    let (cmd_tx, cmd_rx) = mpsc::channel::<ws::WsCommand>(32);
    let (event_tx, mut event_rx) = mpsc::channel::<ws::WsEvent>(32);
    let ws_cfg = ws::WsConfig {
        url: cfg.server_url.clone(),
        room_id: cfg.room_id.clone(),
        token: cfg.token.clone(),
        source_machine: hostname(),
    };
    let proxy_for_ws = event_loop.create_proxy();
    thread::Builder::new()
        .name("clipsync-ws".into())
        .spawn(move || {
            let rt = TokioRtBuilder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let forwarder = tokio::spawn(async move {
                    while let Some(ev) = event_rx.recv().await {
                        let _ = proxy_for_ws.send_event(UserEvent::Ws(ev));
                    }
                });
                ws::run(ws_cfg, cmd_rx, event_tx).await;
                forwarder.abort();
            });
        })
        .context("ws thread başlatılamadı")?;

    // Tray + menü
    let menu = Menu::new();
    let item_status = MenuItem::new("Durum Bilgisi", true, None);
    let item_reconnect = MenuItem::new("Şimdi Yeniden Bağlan", true, None);
    let item_quit = MenuItem::new("Çıkış", true, None);
    menu.append_items(&[
        &item_status,
        &item_reconnect,
        &PredefinedMenuItem::separator(),
        &item_quit,
    ])
    .context("menü oluşturulamadı")?;

    let id_status = item_status.id().clone();
    let id_reconnect = item_reconnect.id().clone();
    let id_quit = item_quit.id().clone();

    let icon = make_default_icon().context("ikon oluşturulamadı")?;
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip(format!("ClipSync — {}", cfg.room_id))
        .build()
        .context("tray oluşturulamadı")?;

    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some({
        let proxy = proxy.clone();
        move |event| {
            let _ = proxy.send_event(UserEvent::Tray(event));
        }
    }));
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let _hidden = hidden;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Menu(ev)) => {
                if ev.id == id_quit {
                    *control_flow = ControlFlow::Exit;
                } else if ev.id == id_status {
                    show_status(&cfg, &engine);
                } else if ev.id == id_reconnect {
                    let _ = cmd_tx.try_send(ws::WsCommand::Reconnect);
                }
            }
            Event::UserEvent(UserEvent::Tray(tray_ev)) => {
                if matches!(tray_ev, TrayIconEvent::DoubleClick { .. }) {
                    show_status(&cfg, &engine);
                }
            }
            Event::UserEvent(UserEvent::Clipboard(text)) => {
                let n = text.chars().count();
                let preview: String = text.chars().take(60).collect();
                let ell = if n > 60 { "…" } else { "" };
                eprintln!(
                    "[clipsync] pano değişti ({} byte, {} char): {:?}{}",
                    text.len(),
                    n,
                    preview,
                    ell
                );
                match engine.on_local_clipboard(text) {
                    LocalAction::Send(t) => {
                        if let Err(e) = cmd_tx.try_send(ws::WsCommand::SendClip(t)) {
                            eprintln!("[clipsync] ws cmd kuyruğu: {e}");
                        } else {
                            eprintln!("[clipsync] → gönderildi");
                        }
                    }
                    LocalAction::Queued => {
                        eprintln!("[clipsync] → bekletildi (bağlantı yok)");
                    }
                    LocalAction::Ignored => {
                        eprintln!("[clipsync] → yutuldu (aynı metin, döngü engeli)");
                    }
                }
            }
            Event::UserEvent(UserEvent::Ws(ev)) => match ev {
                ws::WsEvent::Connected => {
                    eprintln!("[clipsync] WS: bağlandı");
                    if let Some(to_send) = engine.on_connected() {
                        let _ = cmd_tx.try_send(ws::WsCommand::SendClip(to_send));
                        eprintln!("[clipsync] → bekleyen gönderildi");
                    }
                }
                ws::WsEvent::Disconnected => {
                    eprintln!("[clipsync] WS: koptu");
                    engine.on_disconnected();
                }
                ws::WsEvent::Reconnecting { in_ms } => {
                    eprintln!("[clipsync] WS: {in_ms}ms sonra yeniden denenecek");
                    engine.on_reconnecting(in_ms);
                }
                ws::WsEvent::Clip {
                    text,
                    source_machine,
                    timestamp,
                } => {
                    let n = text.chars().count();
                    let preview: String = text.chars().take(60).collect();
                    let ell = if n > 60 { "…" } else { "" };
                    eprintln!(
                        "[clipsync] WS: clip from {} @ {}: {:?}{}",
                        source_machine, timestamp, preview, ell
                    );
                    if let Some(to_write) = engine.on_remote_clip(text, source_machine) {
                        if let Err(e) = clipboard::write_text(&to_write) {
                            eprintln!("[clipsync] pano yazma hatası: {e}");
                        } else {
                            eprintln!("[clipsync] → panoya yazıldı");
                        }
                    } else {
                        eprintln!("[clipsync] → yutuldu (kendi mesajımız veya aynı metin)");
                    }
                }
            },
            _ => {}
        }
    })
}

fn show_status(cfg: &AppConfig, eng: &SyncEngine) {
    let body = format!(
        "Makine          : {}\n\
         Sunucu          : {}\n\
         Oda             : {}\n\
         Bağlantı        : {}\n\
         Son başarı      : {}\n\
         Son işlem       : {}",
        hostname(),
        cfg.server_url,
        cfg.room_id,
        eng.conn_label(),
        eng.last_success_str(),
        eng.last_action(),
    );
    show_dialog("ClipSync", &body);
}

pub fn show_dialog(title: &str, body: &str) {
    let title = HSTRING::from(title);
    let body = HSTRING::from(body);
    unsafe {
        MessageBoxW(
            None,
            &body,
            &title,
            MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONINFORMATION.0),
        );
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "?".into())
}

/// 32x32 boyutunda saydam zemin üstüne mavi "C" çiz (Material Blue 500).
fn make_default_icon() -> Result<tray_icon::Icon> {
    const W: u32 = 32;
    const CENTER: f32 = 15.5;
    const OUTER: f32 = 14.5;
    const INNER: f32 = 9.5;
    let blue = [0x21u8, 0x96, 0xf3, 0xff];

    let mut rgba = vec![0u8; (W * W * 4) as usize];
    for y in 0..W {
        for x in 0..W {
            let dx = x as f32 - CENTER;
            let dy = y as f32 - CENTER;
            let d = (dx * dx + dy * dy).sqrt();
            // Annulus + sağ tarafta ~80° açıklık (C'nin ağzı)
            let in_ring = d >= INNER && d <= OUTER;
            let in_opening = dx > 0.0 && dy.abs() < dx * 0.85;
            if in_ring && !in_opening {
                let idx = ((y * W + x) * 4) as usize;
                rgba[idx..idx + 4].copy_from_slice(&blue);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, W, W).map_err(anyhow::Error::from)
}
