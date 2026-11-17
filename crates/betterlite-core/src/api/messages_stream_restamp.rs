use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

pub mod cache_stream;
mod python_text;
pub mod replay;
pub mod restamp;
mod tests;

// LiteLLM's CACHED_STREAM_EVENTS_KEY, the one key of a cached Messages stream payload
pub const CACHED_STREAM_EVENTS_KEY: &str = "litellm_cached_anthropic_sse_events";

// One physical piece of the /v1/messages stream as LiteLLM's restamper sees it: a decoded event, transport bytes or text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MessagesStreamChunk {
    Event(Json),
    Bytes(Vec<u8>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesStreamRestampInput {
    pub requested_model: String,
    pub chunks: Vec<MessagesStreamChunk>,
}

// the restamped chunk each input chunk produced, then what flush held back for the stream's end
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesStreamRestampResult {
    pub chunks: Vec<MessagesStreamChunk>,
    pub flushed: MessagesStreamChunk,
}

// LiteLLM's AnthropicStreamModelRestamper: every chunk goes through process, then flush drains the held tail
#[must_use]
pub fn restamp_messages_stream(input: &MessagesStreamRestampInput) -> MessagesStreamRestampResult {
    let mut restamper = restamp::StreamModelRestamper::new(input.requested_model.clone());
    let chunks = input
        .chunks
        .iter()
        .map(|chunk| restamper.process(chunk))
        .collect();
    MessagesStreamRestampResult {
        chunks,
        flushed: restamper.flush(),
    }
}
