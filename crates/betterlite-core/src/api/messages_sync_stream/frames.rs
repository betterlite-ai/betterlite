use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::api::messages_stream_usage::state::StreamState;
use crate::api::messages_stream_usage::usage::initial_usage;

// message_start announces the message and its four zero cache counters before any provider read.
pub(super) fn message_start<S>(state: &StreamState<S>) -> Object {
    let mut message = Object::new();
    message.insert("id".to_owned(), Json::Text(state.message_id.clone()));
    message.insert("type".to_owned(), Json::Text("message".to_owned()));
    message.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    message.insert("content".to_owned(), Json::List(Vec::new()));
    message.insert("model".to_owned(), Json::Text(state.model.clone()));
    message.insert("stop_reason".to_owned(), Json::Null);
    message.insert("stop_sequence".to_owned(), Json::Null);
    message.insert("usage".to_owned(), Json::Object(initial_usage()));
    let mut event = Object::new();
    event.insert("type".to_owned(), Json::Text("message_start".to_owned()));
    event.insert("message".to_owned(), Json::Object(message));
    event
}

// Every content-block frame names its type and the index its block occupies.
pub(super) fn block_frame(kind: &str, index: i64) -> Object {
    let mut event = Object::new();
    event.insert("type".to_owned(), Json::Text(kind.to_owned()));
    event.insert("index".to_owned(), Json::Number(Number::from(index)));
    event
}

// The start body is the block B976's should-start step last recorded, not the chunk that triggered it.
pub(super) fn content_block_start(index: i64, block: &Json) -> Object {
    let mut event = block_frame("content_block_start", index);
    event.insert("content_block".to_owned(), block.clone());
    event
}

pub(super) fn message_stop() -> Object {
    let mut event = Object::new();
    event.insert("type".to_owned(), Json::Text("message_stop".to_owned()));
    event
}
