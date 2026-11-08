use creation::{StreamInstant, system_instant};
use records::{JsonObject, StreamChunk};
use thiserror::Error;
use uuid::Uuid;

pub mod creation;
pub mod decision;
pub mod extras;
pub mod records;
mod tests;
pub mod tokens;
pub mod usage_totals;

// The wrapper raises litellm.InternalServerError for a repeating model, which the proxy answers with a 500.
#[derive(Debug, Error)]
pub enum StreamShapeError {
    #[error("The model is repeating the same chunk = {last_content}.")]
    Repetition {
        last_content: String,
        model: String,
        provider: String,
    },
    #[error("the stream chunk arguments are invalid: {source}")]
    Arguments {
        #[from]
        source: serde_json::Error,
    },
}

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
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

impl ChunkShaper {
    #[must_use]
    pub fn new(
        model: String,
        provider: Option<String>,
        merge_reasoning_in_choices: bool,
        base_hidden_params: JsonObject,
    ) -> Self {
        Self {
            model,
            provider,
            merge_reasoning_in_choices,
            tool_call: false,
            base_hidden_params,
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
            now: system_instant,
            next_id: Uuid::new_v4,
        }
    }
}
