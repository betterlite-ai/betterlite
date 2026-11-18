use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;

pub mod fallback;
pub mod frames;
pub mod restamp;
pub mod serialize;

pub const DONE_FRAME: &str = "data: [DONE]\n\n";
pub const SSE_FRAME_DELIMITERS: [&str; 3] = ["\r\n\r\n", "\n\n", "\r\r"];
pub const MAX_RAW_SSE_BUFFER_CHARS: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    ModelStream,
    ModelFull,
    Event,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChunk {
    pub body: JsonObject,
    pub kind: ChunkKind,
    pub hidden: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    Chunk(StreamChunk),
    Bytes(Bytes),
    Text(String),
}

impl StreamItem {
    #[must_use]
    pub const fn hidden_params(&self) -> Option<&JsonObject> {
        match self {
            Self::Chunk(chunk) => Some(&chunk.hidden),
            Self::Bytes(_) | Self::Text(_) => None,
        }
    }
}

mod tests;
