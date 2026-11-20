use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_openai::text_completion_transport::sdk::ByteStream;
use betterlite_openai::text_completion_transport::stream::{
    CompletionStream, StreamSetup, completion_stream,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamOptions};
use uuid::Uuid;

use crate::azure_stream_errors::stream as chat_stream;

pub const AZURE_TEXT_PROVIDER: &str = "azure_text";
// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
pub const REPEATED_STREAMING_CHUNK_LIMIT: usize = 100;

pub struct StreamInput<'stream> {
    pub model: &'stream str,
    pub data: &'stream Extra,
    pub defaults: StreamDefaults,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

// The Azure text adapter reads an explicit empty object as include_usage off, so that is its fallback.
pub(super) fn requested_stream_options(
    data: &Extra,
    defaults: Option<StreamOptions>,
) -> Option<StreamOptions> {
    let empty_object = matches!(
        data.get("stream_options"),
        Some(Json::Object(fields)) if fields.is_empty()
    );
    let fallback = if empty_object {
        Some(StreamOptions {
            include_usage: false,
        })
    } else {
        defaults
    };
    chat_stream::requested_stream_options(data, fallback)
}

#[must_use]
pub fn wrap(bytes: ByteStream, input: StreamInput<'_>) -> CompletionStream {
    let mut base_hidden_params = input.defaults.base_hidden_params;
    base_hidden_params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::String(AZURE_TEXT_PROVIDER.to_owned()),
    );
    let setup = StreamSetup {
        model: input.model.to_owned(),
        stream_options: requested_stream_options(input.data, input.defaults.stream_options),
        merge_reasoning_in_choices: input.defaults.merge_reasoning_in_choices,
        is_function_call: input.defaults.is_function_call,
        base_hidden_params,
        repeated_chunk_limit: REPEATED_STREAMING_CHUNK_LIMIT,
        now: input.now,
        next_id: input.next_id,
    };
    completion_stream(bytes, setup)
}
