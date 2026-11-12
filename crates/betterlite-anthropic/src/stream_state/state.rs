use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;

use super::json_fragment_buffer::JsonFragmentBuffer;
use super::wire::{ContentBlockDelta, JsonFields};

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

pub struct UuidV4;

impl IdSource for UuidV4 {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    // LiteLLM stores the flag and never reads it in this slice
    pub sync_stream: bool,
    pub json_mode: Option<bool>,
    pub speed: Option<String>,
    pub tool_name_reverse_map: IndexMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkType {
    ValidJson,
    AccumulatedJson,
}

#[derive(Debug, Clone)]
pub struct StreamState {
    pub content_blocks: Vec<ContentBlockDelta>,
    pub tool_index: i64,
    pub json_mode: Option<bool>,
    pub speed: Option<String>,
    pub cumulative_usage: JsonFields,
    pub tool_name_reverse_map: IndexMap<String, String>,
    pub response_id: String,
    pub served_model: Option<String>,
    pub is_response_format_tool: bool,
    pub converted_response_format_tool: bool,
    pub json_buffer: JsonFragmentBuffer,
    pub chunk_type: ChunkType,
    pub current_content_block_type: Option<String>,
    pub web_search_results: Vec<Json>,
    pub web_search_calls: IndexMap<String, Json>,
    pub compaction_blocks: Vec<Json>,
    pub reasoning_content_chunks: Vec<String>,
    pub server_tool_inputs: IndexMap<String, Json>,
    pub tool_results: Vec<Json>,
    pub current_server_tool_id: Option<String>,
    pub container_id: Option<String>,
}

impl StreamState {
    #[must_use]
    pub fn new(options: StreamOptions, ids: &dyn IdSource) -> Self {
        let response_id = format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id());
        Self {
            content_blocks: Vec::new(),
            tool_index: -1,
            json_mode: options.json_mode,
            speed: options.speed,
            cumulative_usage: JsonFields::new(),
            tool_name_reverse_map: options.tool_name_reverse_map,
            response_id,
            served_model: None,
            is_response_format_tool: false,
            converted_response_format_tool: false,
            json_buffer: JsonFragmentBuffer::default(),
            chunk_type: ChunkType::ValidJson,
            current_content_block_type: None,
            web_search_results: Vec::new(),
            web_search_calls: IndexMap::new(),
            compaction_blocks: Vec::new(),
            reasoning_content_chunks: Vec::new(),
            server_tool_inputs: IndexMap::new(),
            tool_results: Vec::new(),
            current_server_tool_id: None,
            container_id: None,
        }
    }
}

#[must_use]
pub fn check_empty_tool_call_args(state: &StreamState) -> bool {
    let Some(first) = state.content_blocks.first() else {
        return false;
    };
    if first.delta.kind == "text_delta" || first.delta.kind == "thinking_delta" {
        return false;
    }
    let mut arguments = String::new();
    for block in &state.content_blocks {
        if block.delta.kind == "input_json_delta"
            && let Some(partial) = &block.delta.partial_json
        {
            arguments.push_str(partial);
        }
    }
    arguments.is_empty()
}
