use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use indexmap::IndexMap;
use serde::Serialize;

use crate::agentcore_stream::IdSource;
use crate::converse_events::response::ConverseChunk;

mod dispatch;
pub mod error;
mod events;
mod projection;
pub mod response;
mod state;
pub mod thinking;
pub mod wire;

mod tests;

pub struct ConverseStreamState {
    pub content_blocks: Vec<Json>,
    pub tool_calls_index: Option<i64>,
    pub response_id: Option<String>,
    pub current_tool_name: Option<String>,
    pub thinking_ran: bool,
    pub provider_reasoning_tokens: Option<i64>,
    pub json_mode: bool,
    pub model: String,
    pub mappings: IndexMap<String, String>,
    pub ids: Arc<dyn IdSource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkProjection {
    Converse(ConverseChunk),
    Generic(GenericChunk),
}
