mod config;
mod room;
mod wire;
mod ws;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::ServerConfig;
use crate::room::Rooms;

pub struct AppState {
    pub auth: HashMap<String, String>, // room_id -> token
    pub rooms: Rooms,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let cfg = ServerConfig::load(&config_path)
        .with_context(|| format!("config yüklenemedi: {}", config_path.display()))?;

    let bind = cfg.bind.clone();
    let state = Arc::new(AppState {
        auth: cfg.auth_map(),
        rooms: Rooms::default(),
    });

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .with_state(state);

    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("port bağlanamadı: {bind}"))?;

    info!(%bind, rooms = cfg.rooms.len(), "clipsync-server dinliyor");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("sunucu hatası")?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown sinyali alındı");
}
