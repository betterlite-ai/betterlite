#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        clippy::needless_pass_by_value,
        clippy::shadow_reuse,
        reason = "the shared harness takes fixture JSON and captured events by value"
    )
)]

use std::fmt;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tracing::field::Field as EventField;
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;
use crate::api::responses_to_chat_stream::records::{ChatChunk, ChunkFactory};
use crate::api::responses_to_chat_stream::stream::{
    ResponsesChatStream, get_model_response_iterator,
};
use crate::api::responses_to_chat_stream::translate::translate_event;

pub(super) const FIXED_ID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
pub(super) const FIXED_CLOCK: i64 = 1_700_000_000;
pub(super) const GENERATED_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";

pub(super) fn fixed_identifier() -> Uuid {
    Uuid::from_u128(FIXED_ID)
}

pub(super) fn fixed_clock() -> i64 {
    FIXED_CLOCK
}

pub(super) fn factory() -> ChunkFactory {
    ChunkFactory {
        identifier: fixed_identifier,
        clock: fixed_clock,
    }
}

pub(super) fn stream() -> ResponsesChatStream {
    get_model_response_iterator(Some(false), fixed_identifier, fixed_clock)
}

pub(super) fn event(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub(super) fn rendered(chunk: &ChatChunk) -> Value {
    serde_json::to_value(chunk).unwrap()
}

pub(super) fn translated(chunk: &Value) -> Result<Value, ResponsesChatStreamError> {
    translate_event(chunk, None, factory()).map(|chunk| rendered(&chunk))
}

// LiteLLM's Delta carries these five keys on every chunk, null where unset
pub(super) fn empty_delta() -> Value {
    json!({"content": "", "role": null, "function_call": null, "tool_calls": null, "audio": null})
}

pub(super) fn choice(delta: Value, finish_reason: Value, index: Value) -> Value {
    json!({"index": index, "delta": delta, "finish_reason": finish_reason, "logprobs": null})
}

pub(super) fn chunk_of(id: &str, choices: Value) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": FIXED_CLOCK,
        "choices": choices,
        "provider_specific_fields": null,
    })
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |_field: &EventField, value: &dyn fmt::Debug| {
            message = format!("{value:?}");
        });
        self.0.lock().unwrap().push(message);
    }
}

pub(super) fn capture<Output, Call>(call: Call) -> (Output, Vec<String>)
where
    Call: FnOnce() -> Output,
{
    let captured = Captured::default();
    let subscriber = registry().with(captured.clone());
    let output = with_default(subscriber, call);
    let lines = captured.0.lock().unwrap().clone();
    (output, lines)
}

pub(super) fn padding_value() -> Value {
    json!({
        "text": "",
        "tool_use": null,
        "is_finished": false,
        "finish_reason": "",
        "usage": null,
    })
}
