use chrono::{DateTime, Local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connected,
    Reconnecting,
    Disconnected,
}

/// Brief'teki davranışı uygular:
/// - `last_local_text` invariant'ı ile sonsuz döngü engeli
/// - Bağlantı kopukken son metin `pending`'de tutulur (tek slot)
/// - Sunucudan gelen kendi mesajımız ya da aynı metin yutulur
#[derive(Debug)]
pub enum LocalAction {
    Send(String),
    Queued,
    Ignored,
}

pub struct SyncEngine {
    source_machine: String,
    last_local_text: Option<String>,
    pending: Option<String>,
    conn: ConnState,
    last_success: Option<DateTime<Local>>,
    last_action: String,
}

impl SyncEngine {
    pub fn new(source_machine: String, initial_local: Option<String>) -> Self {
        Self {
            source_machine,
            last_local_text: initial_local,
            pending: None,
            conn: ConnState::Disconnected,
            last_success: None,
            last_action: "başlatıldı".into(),
        }
    }

    /// Yerel WM_CLIPBOARDUPDATE.
    pub fn on_local_clipboard(&mut self, text: String) -> LocalAction {
        if self.last_local_text.as_deref() == Some(text.as_str()) {
            // Genelde panoya kendi yazdığımız metnin geri yansıması — yut.
            return LocalAction::Ignored;
        }
        self.last_local_text = Some(text.clone());
        match self.conn {
            ConnState::Connected => {
                self.last_action = "pano değişti → gönderildi".into();
                self.last_success = Some(Local::now());
                LocalAction::Send(text)
            }
            _ => {
                self.pending = Some(text);
                self.last_action = "pano değişti → bağlantı yok, bekletildi".into();
                LocalAction::Queued
            }
        }
    }

    /// Sunucudan gelen clip. None döndürürse panoya yazılmaz.
    pub fn on_remote_clip(&mut self, text: String, source_machine: String) -> Option<String> {
        if source_machine == self.source_machine {
            // Kendi mesajımız — sunucu normalde göndermez ama emniyet.
            return None;
        }
        if self.last_local_text.as_deref() == Some(text.as_str()) {
            return None;
        }
        // last_local_text'i panoya yazmadan ÖNCE güncelliyoruz; yazma sonrası
        // tetiklenen WM_CLIPBOARDUPDATE'te aynı metin görüldüğünde döngü kırılır.
        self.last_local_text = Some(text.clone());
        self.last_action = format!("{} → panoya yazıldı", source_machine);
        self.last_success = Some(Local::now());
        Some(text)
    }

    /// WS bağlandığında. Pending varsa onu da yollatır.
    pub fn on_connected(&mut self) -> Option<String> {
        self.conn = ConnState::Connected;
        if let Some(text) = self.pending.take() {
            self.last_action = "bağlantı kuruldu → bekleyen gönderildi".into();
            self.last_success = Some(Local::now());
            Some(text)
        } else {
            self.last_action = "bağlantı kuruldu".into();
            None
        }
    }

    pub fn on_disconnected(&mut self) {
        self.conn = ConnState::Disconnected;
        self.last_action = "bağlantı koptu".into();
    }

    pub fn on_reconnecting(&mut self, in_ms: u64) {
        self.conn = ConnState::Reconnecting;
        self.last_action = format!("{}ms sonra yeniden denenecek", in_ms);
    }

    pub fn conn_label(&self) -> &'static str {
        match self.conn {
            ConnState::Connected => "Bağlı",
            ConnState::Reconnecting => "Bağlanıyor",
            ConnState::Disconnected => "Kopuk",
        }
    }

    pub fn last_success_str(&self) -> String {
        self.last_success
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".into())
    }

    pub fn last_action(&self) -> &str {
        &self.last_action
    }
}
