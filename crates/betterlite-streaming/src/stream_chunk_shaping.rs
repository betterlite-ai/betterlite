use std::sync::Arc;

use creation::StreamIdentity;
use records::{JsonObject, StreamChunk};

pub mod creation;
pub mod decision;
pub mod error;
pub mod generic;
pub mod records;
pub mod specials;
pub mod sse;
mod tests;
pub mod tokens;
pub mod usage;
pub mod usage_totals;

// The two send flags stay apart: a finish reason can arrive before any chunk was sent.
pub struct ChunkProgress {
    pub sent_first_chunk: bool,
    pub sent_last_chunk: bool,
}

pub struct ThinkingProgress {
    pub sent_first_block: bool,
    pub sent_last_block: bool,
}

// Everything CustomStreamWrapper keeps on self that the shaping rules read or write.
pub struct ChunkShaper {
    pub model: String,
    pub provider: Option<String>,
    pub merge_reasoning_in_choices: bool,
    pub tool_call: bool,
    pub base_hidden_params: JsonObject,
    pub response_id: Option<String>,
    pub created: Option<i64>,
    pub system_fingerprint: Option<String>,
    pub progress: ChunkProgress,
    pub thinking: ThinkingProgress,
    pub holding_chunk: String,
    pub response_uptil_now: String,
    pub received_finish_reason: Option<String>,
    pub intermittent_finish_reason: Option<String>,
    pub chunks: Vec<StreamChunk>,
    pub repeated_messages_count: usize,
    pub identity: Arc<dyn StreamIdentity>,
}

// The wrapper's constructor arguments; the identity source replaces Python's uuid4 and time.time.
pub struct ChunkShaperConfig {
    pub model: String,
    pub provider: Option<String>,
    pub merge_reasoning_in_choices: bool,
    pub base_hidden_params: JsonObject,
    pub identity: Arc<dyn StreamIdentity>,
}

impl From<ChunkShaperConfig> for ChunkShaper {
    fn from(config: ChunkShaperConfig) -> Self {
        Self {
            model: config.model,
            provider: config.provider,
            merge_reasoning_in_choices: config.merge_reasoning_in_choices,
            tool_call: false,
            base_hidden_params: config.base_hidden_params,
            response_id: None,
            created: None,
            system_fingerprint: None,
            progress: ChunkProgress {
                sent_first_chunk: false,
                sent_last_chunk: false,
            },
            thinking: ThinkingProgress {
                sent_first_block: false,
                sent_last_block: false,
            },
            holding_chunk: String::new(),
            response_uptil_now: String::new(),
            received_finish_reason: None,
            intermittent_finish_reason: None,
            chunks: Vec::new(),
            repeated_messages_count: 1,
            identity: config.identity,
        }
    }
}
