use std::collections::VecDeque;

use betterlite_logging::logging_state::LoggingState;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;

use crate::api::messages_context_clear::result::{AppliedEdit, CompactionBlock, UsageIteration};
use crate::api::messages_stream_split::ChunkSplitter;

// LiteLLM's current_content_block_type, the three kinds the streaming translate step can start
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Text,
    ToolUse,
    Thinking,
}

// message_start and message_stop are independent sends, since a finish reason can arrive before either
#[derive(Default)]
pub struct MessageProgress {
    pub sent_first_chunk: bool,
    pub sent_last_message: bool,
}

// A finish reason closes the open content block once, and a new block may open afterwards
#[derive(Default)]
pub struct BlockProgress {
    pub sent_content_block_start: bool,
    pub sent_content_block_finish: bool,
}

// The synthetic compaction block hands out its start and delta on separate calls
#[derive(Default)]
pub struct CompactionProgress {
    pub sent_compaction_block_start: bool,
    pub sent_compaction_block_delta: bool,
}

// The polyfill output and tool-name corrections the call was translated with, copied so later edits cannot leak in
pub struct StreamSetup {
    pub model: String,
    pub tool_name_mapping: IndexMap<String, String>,
    pub applied_edits: Vec<AppliedEdit>,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
}

// Everything AnthropicStreamWrapper keeps on self, one instance per stream so concurrent streams share nothing
pub struct StreamState<S> {
    pub model: String,
    pub message_id: String,
    pub tool_name_mapping: IndexMap<String, String>,
    pub applied_edits: Vec<AppliedEdit>,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
    pub refusal_text: String,
    pub completion_stream: ChunkSplitter<S>,
    pub chunk_queue: VecDeque<Object>,
    pub current_content_block_index: i64,
    pub current_content_block_type: BlockKind,
    pub current_content_block_start: Json,
    pub holding_chunk: Option<Object>,
    pub holding_stop_reason_chunk: Option<Object>,
    pub queued_usage_chunk: bool,
    pub sent_compaction_block: bool,
    pub message_progress: MessageProgress,
    pub block_progress: BlockProgress,
    pub compaction_progress: CompactionProgress,
}

impl<S> StreamState<S> {
    // The constructor owns the id, the logging record and the default block; the caller names every other field
    pub fn new(logging: Option<&mut LoggingState>, source: S, setup: StreamSetup) -> Self {
        let message_id = format!("msg_{}", Uuid::new_v4());
        if let Some(recorder) = logging {
            recorder.streamed_anthropic_message_id = Some(message_id.clone());
        }
        let mut start = Object::new();
        start.insert("type".to_owned(), Json::Text("text".to_owned()));
        start.insert("text".to_owned(), Json::Text(String::new()));
        Self {
            model: setup.model,
            message_id,
            tool_name_mapping: setup.tool_name_mapping,
            applied_edits: setup.applied_edits,
            compaction_block: setup.compaction_block,
            iterations_usage: setup.iterations_usage,
            refusal_text: String::new(),
            completion_stream: ChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            chunk_queue: VecDeque::new(),
            current_content_block_index: 0,
            current_content_block_type: BlockKind::Text,
            current_content_block_start: Json::Object(start),
            holding_chunk: None,
            holding_stop_reason_chunk: None,
            queued_usage_chunk: false,
            sent_compaction_block: false,
            message_progress: MessageProgress::default(),
            block_progress: BlockProgress::default(),
            compaction_progress: CompactionProgress::default(),
        }
    }
}
