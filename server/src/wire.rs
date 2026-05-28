use serde::{Deserialize, Serialize};

pub const MAX_CLIP_BYTES: usize = 1024 * 1024; // 1 MiB

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WireMessage {
    Clip {
        text: String,
        source_machine: String,
        timestamp: String,
    },
    Ping,
    Pong,
}
