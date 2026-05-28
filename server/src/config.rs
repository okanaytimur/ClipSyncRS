use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    #[serde(default)]
    pub rooms: Vec<RoomConfig>,
}

#[derive(Debug, Deserialize)]
pub struct RoomConfig {
    pub id: String,
    pub token: String,
}

impl ServerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("config dosyası okunamadı: {}", path.display()))?;
        let cfg: ServerConfig = toml::from_str(&text)
            .with_context(|| format!("config parse hatası: {}", path.display()))?;
        Ok(cfg)
    }

    pub fn auth_map(&self) -> HashMap<String, String> {
        self.rooms
            .iter()
            .map(|r| (r.id.clone(), r.token.clone()))
            .collect()
    }
}
