#![cfg(test)]

use std::fmt::Debug;
use std::iter::repeat_n;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::stream_chunk_shaping::creation::{
    StreamInstant, copy_provider_specific_fields, model_response_creator,
    parsed_provider_hidden_params, provider_hidden_params, provider_response_model, set_model_id,
    stamp_served_service_tier,
};
use crate::stream_chunk_shaping::decision::{
    combine_thinking_in_choices, is_chunk_non_empty, is_delta_empty, return_processed_chunk_logic,
};
use crate::stream_chunk_shaping::extras::{
    SpecialDeltaAttribute, apply_special_delta_attributes, convert_generic_chunk,
    copy_delta_attribute, generic_chunk_has_known_fields, has_any_special_attributes,
    has_special_delta_attribute, has_special_delta_content, strip_sse_data_from_chunk,
};
use crate::stream_chunk_shaping::records::{
    ChunkResponseObject, GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk,
    StreamDelta, StreamOptions, TokenDetails, Usage,
};
use crate::stream_chunk_shaping::tokens::{
    check_is_function_call, check_send_stream_usage, check_special_tokens, finish_reason_handler,
    mapped_finish_reason, raise_on_model_repetition,
};
use crate::stream_chunk_shaping::usage_totals::{
    backfill_cache_usage, cache_token_count, calculate_total_usage, coerce_token_details,
    propagate_usage_cost, record_usage_only_chunk, resolve_provider_reported_cost,
};
use crate::stream_chunk_shaping::{ChunkShaper, StreamShapeError};

fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

const BASE_HIDDEN: &str = r#"{"model_id": "m1", "api_base": "https://example.test", "additional_headers": {}, "response_cost": null}"#;

fn shaper(model: &str, provider: Option<&str>) -> ChunkShaper {
    let mut shaper = ChunkShaper::new(
        model.to_owned(),
        provider.map(str::to_owned),
        false,
        json_object(BASE_HIDDEN),
    );
    shaper.now = fixed_instant;
    shaper
}

fn parse<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(source).expect("the test literal is valid")
}

fn json_object(source: &str) -> JsonObject {
    parse(source)
}

fn number(value: &str) -> JsonValue {
    JsonValue::Number(value.parse().expect("a JSON number literal"))
}

fn empty_delta() -> StreamDelta {
    StreamDelta {
        content: None,
        role: None,
        function_call: None,
        tool_calls: None,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content: None,
        thinking_blocks: None,
        provider_specific_fields: None,
        extra: JsonObject::new(),
    }
}

fn content_delta(text: &str) -> StreamDelta {
    let mut delta = empty_delta();
    delta.content = Some(text.to_owned());
    delta
}

fn details(source: &str) -> TokenDetails {
    parse(source)
}

fn choice(delta: StreamDelta) -> StreamChoice {
    StreamChoice {
        index: 0,
        delta,
        finish_reason: None,
        logprobs: None,
        extra: JsonObject::new(),
    }
}

fn chunk(id: &str, created: i64) -> StreamChunk {
    StreamChunk {
        id: id.to_owned(),
        object: "chat.completion.chunk".to_owned(),
        created,
        model: "gpt-4".to_owned(),
        system_fingerprint: None,
        choices: vec![choice(empty_delta())],
        usage: None,
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn chunk_with_content(id: &str, text: &str) -> StreamChunk {
    let mut response = chunk(id, 1);
    response.choices = vec![choice(content_delta(text))];
    response
}

fn usage_with(prompt: i64, completion: i64) -> Usage {
    let total = prompt.saturating_add(completion);
    parse(&format!(
        r#"{{"prompt_tokens": {prompt}, "completion_tokens": {completion}, "total_tokens": {total}}}"#
    ))
}

fn usage_chunk(usage: Usage) -> StreamChunk {
    let mut response = chunk("c", 1);
    response.usage = Some(usage);
    response
}

fn creator(shaper: &mut ChunkShaper, source: &str, hidden: Option<&str>) -> StreamChunk {
    let arguments = json_object(source);
    let hidden_params = hidden.map(json_object);
    model_response_creator(shaper, Some(&arguments), hidden_params.as_ref())
        .expect("the chunk arguments are valid")
}

include!("tests/creation.rs");
include!("tests/decision.rs");
include!("tests/tokens.rs");
include!("tests/usage.rs");
include!("tests/generic.rs");
