use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppConfig {
    pub server_url: String,
    pub room_id: String,
    pub token: String,
}

impl AppConfig {
    pub fn template() -> Self {
        Self {
            server_url: "wss://clip.ornek.com/ws".into(),
            room_id: "okan-home".into(),
            token: "uzun-rastgele-string-buraya".into(),
        }
    }

    /// Kullanıcı henüz düzenlememiş, hâlâ örnek değerler içeriyor.
    pub fn is_template(&self) -> bool {
        let t = Self::template();
        self.server_url == t.server_url
            || self.token == t.token
    }

    pub fn validate(&self) -> Result<()> {
        if !(self.server_url.starts_with("ws://") || self.server_url.starts_with("wss://")) {
            anyhow::bail!("ServerUrl `ws://` veya `wss://` ile başlamalı: {}", self.server_url);
        }
        if self.room_id.trim().is_empty() {
            anyhow::bail!("RoomId boş olamaz");
        }
        if self.token.trim().is_empty() {
            anyhow::bail!("Token boş olamaz");
        }
        Ok(())
    }
}

pub fn config_path() -> Result<PathBuf> {
    let mut p = std::env::current_exe().context("current_exe alınamadı")?;
    p.pop();
    p.push("config.json");
    Ok(p)
}

pub enum FirstRun {
    /// Yüklendi ve doğrulandı.
    Loaded(AppConfig),
    /// Config yoktu; şablon yazıldı. Kullanıcı düzenlemeli.
    TemplateWritten(PathBuf),
    /// Config var ama hâlâ şablon değerler içeriyor.
    NeedsEdit(PathBuf),
}

pub fn load_or_init() -> Result<FirstRun> {
    let path = config_path()?;
    if !path.exists() {
        let tmpl = AppConfig::template();
        let json = serde_json::to_string_pretty(&tmpl).context("şablon serialize hatası")?;
        std::fs::write(&path, json)
            .with_context(|| format!("şablon yazılamadı: {}", path.display()))?;
        return Ok(FirstRun::TemplateWritten(path));
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("config okunamadı: {}", path.display()))?;
    // Notepad/PowerShell UTF-8-with-BOM yazabiliyor; baştaki BOM'u kırp.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text.as_str());
    let cfg: AppConfig = serde_json::from_str(text)
        .with_context(|| format!("config parse hatası: {}", path.display()))?;
    if cfg.is_template() {
        return Ok(FirstRun::NeedsEdit(path));
    }
    cfg.validate()?;
    Ok(FirstRun::Loaded(cfg))
}
