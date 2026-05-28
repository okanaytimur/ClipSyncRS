use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;
use tracing::warn;

use crate::wire::WireMessage;

pub type RoomId = String;
pub type ClientId = u64;

#[derive(Clone)]
pub struct ClientHandle {
    pub id: ClientId,
    pub tx: mpsc::Sender<WireMessage>,
}

#[derive(Default)]
pub struct RoomState {
    pub clients: Vec<ClientHandle>,
    pub last: Option<WireMessage>, // her zaman Clip variant
}

#[derive(Default)]
pub struct Rooms {
    inner: Mutex<HashMap<RoomId, RoomState>>,
}

impl Rooms {
    /// Odaya katıl. Varsa cache'lenmiş son `clip` mesajı döner — çağıran
    /// istemcinin kendi kuyruğuna iletmesi gerekir.
    pub fn join(&self, room: &str, client: ClientHandle) -> Option<WireMessage> {
        let mut g = self.inner.lock().unwrap();
        let st = g.entry(room.to_string()).or_default();
        st.clients.push(client);
        st.last.clone()
    }

    pub fn leave(&self, room: &str, client_id: ClientId) {
        let mut g = self.inner.lock().unwrap();
        if let Some(st) = g.get_mut(room) {
            st.clients.retain(|c| c.id != client_id);
            // Oda boşalsa bile state'i tutuyoruz — `last` cache'i sonradan
            // bağlanan istemci için faydalı. İleride TTL'lenebilir.
        }
    }

    /// `from` dışındaki tüm odadaki istemcilere yollar. Clip ise `last`'i günceller.
    pub fn broadcast(&self, room: &str, from: ClientId, msg: WireMessage) {
        let recipients: Vec<ClientHandle> = {
            let mut g = self.inner.lock().unwrap();
            let Some(st) = g.get_mut(room) else { return };
            if matches!(msg, WireMessage::Clip { .. }) {
                st.last = Some(msg.clone());
            }
            st.clients.iter().filter(|c| c.id != from).cloned().collect()
        };
        for c in recipients {
            if let Err(e) = c.tx.try_send(msg.clone()) {
                warn!(client_id = c.id, error = %e, "broadcast iletilemedi (kuyruk dolu/kapalı)");
            }
        }
    }
}
