#![cfg(test)]

use std::io::Error;
use std::path::Path;
use std::str::Utf8Error;
use std::{fs, str};

use rstest::rstest;
use serde_json::from_str;
use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, RequestOptions, as_array, as_object,
};
use crate::provider_chunk_decode::creator::chunk_creator;
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::fuzz_input::decode_fuzzed_stream_chunk;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::StreamInstant;
use crate::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk, Usage,
};

fn options() -> RequestOptions {
    RequestOptions {
        send_stream_usage: false,
        is_function_call: false,
    }
}

fn state(model: &str) -> DecodeState {
    DecodeState {
        model: model.to_owned(),
        kind: ProviderKind::Builtin,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: None,
        options: options(),
        repeated_chunk_limit: 100,
    }
}

fn custom_state(model: &str) -> DecodeState {
    let mut decode_state = state(model);
    decode_state.kind = ProviderKind::Custom;
    decode_state
}

fn json(source: &str) -> JsonValue {
    from_str(source).expect("the test literal is valid JSON")
}

fn json_chunk(source: &str) -> ProviderChunk {
    ProviderChunk::Json(Box::new(json(source)))
}

fn generic_chunk(source: &str) -> ProviderChunk {
    let parsed: GenericChunk = from_str(source).expect("the test literal is a generic chunk");
    ProviderChunk::Generic(Box::new(parsed))
}

fn text(value: &str) -> ProviderChunk {
    ProviderChunk::Text(value.to_owned())
}

fn decode(provider: &str, chunk: &ProviderChunk, decode_state: &mut DecodeState) -> DecodedChunk {
    decode_provider_chunk(Some(provider), chunk, decode_state).expect("the chunk decodes")
}

fn decode_error(
    provider: &str,
    chunk: &ProviderChunk,
    decode_state: &mut DecodeState,
) -> ProviderChunkError {
    decode_provider_chunk(Some(provider), chunk, decode_state).expect_err("the chunk fails")
}

fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn shaper(provider: &str) -> ChunkShaper {
    let mut shaping = ChunkShaper::new(
        "gpt-4".to_owned(),
        Some(provider.to_owned()),
        false,
        JsonObject::new(),
    );
    shaping.now = fixed_instant;
    shaping.next_id = fixed_id;
    shaping
}

fn created(
    provider: &str,
    decode_state: &mut DecodeState,
    chunk: &ProviderChunk,
) -> Option<StreamChunk> {
    let mut shaping = shaper(provider);
    chunk_creator(&mut shaping, decode_state, chunk).expect("the chunk is shaped")
}

fn usage(prompt: i64, completion: i64, total: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

fn first_choice(chunk: &StreamChunk) -> &StreamChoice {
    chunk.choices.first().expect("the chunk has a choice")
}

#[rstest]
#[case::stop_iteration(ProviderChunkError::StopIteration, "StopIteration")]
#[case::unknown_stream(ProviderChunkError::UnknownStream, "Exception")]
#[case::vertex_blocked(ProviderChunkError::VertexBlocked, "Exception")]
#[case::not_a_string(
    ProviderChunkError::NotAString { chunk: String::new() },
    "ValueError"
)]
#[case::missing_field(
    ProviderChunkError::MissingField { field: "text" },
    "TypeError"
)]
#[case::invalid_utf8(
    ProviderChunkError::InvalidUtf8 {
        source: invalid_utf8()
    },
    "UnicodeDecodeError"
)]
fn errors_class_names(#[case] error: ProviderChunkError, #[case] class: &str) {
    assert_eq!(error.class(), class);
}

fn invalid_utf8() -> Utf8Error {
    let bytes = vec![0xff_u8];
    str::from_utf8(&bytes).expect_err("the bytes are invalid")
}

fn provider_field_chunk() -> ProviderChunk {
    generic_chunk(
        r#"{"text": "hi", "is_finished": false, "finish_reason": "", "usage": null, "provider_specific_fields": {
            "id": "provider-id",
            "object": "provider.object",
            "model": "provider-model",
            "created": 1234,
            "system_fingerprint": "provider-fp",
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
            "provider_specific_fields": {"nested": 1},
            "hidden_params": {"marker": "h"},
            "choices": [{"index": 3, "delta": {"content": "ignored"}}],
            "unknown_key": "extra"
        }}"#,
    )
}

fn first_tool_call(chunk: &StreamChunk) -> JsonValue {
    let calls = first_choice(chunk)
        .delta
        .tool_calls
        .as_ref()
        .expect("the calls are kept");
    as_array(calls)
        .and_then(|items| items.first())
        .cloned()
        .expect("one call")
}

include!("tests/generic.rs");
include!("tests/text_providers.rs");
include!("tests/openai.rs");
include!("tests/errors.rs");
include!("tests/creator.rs");

#[test]
fn replays_the_provider_stream_chunk_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/provider_stream_chunk");
    let entries: Vec<_> = fs::read_dir(root).expect("the corpus exists").collect();
    assert!(!entries.is_empty());
    entries
        .into_iter()
        .try_for_each(|entry| {
            let path = entry.expect("a corpus entry").path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the seed has a name");
            check_seed(name, decode_fuzzed_stream_chunk(&bytes));
            Ok::<(), Error>(())
        })
        .expect("every seed reads");
}

fn check_seed(name: &str, seed: Option<Result<DecodedChunk, ProviderChunkError>>) {
    let outcome = seed.expect("the seed has a provider byte");
    match name {
        "openai-text" => assert_eq!(outcome.expect("the seed decodes").text, "hi"),
        "generic-text" => assert_eq!(outcome.expect("the seed decodes").text, "generic"),
        "replicate-succeeded" => assert!(outcome.expect("the seed decodes").is_finished),
        "custom-openai" => assert!(outcome.expect("the seed decodes").early_return),
        "vertex-blocked" => assert!(matches!(
            outcome.expect_err("the seed fails"),
            ProviderChunkError::VertexBlocked
        )),
        "petals-text" => assert_eq!(
            outcome.expect("the seed decodes").text,
            "abcdefghijklmnopqrstuvwxyzabcd"
        ),
        "nlp-cloud-error" => assert!(matches!(
            outcome.expect_err("the seed fails"),
            ProviderChunkError::UnknownStream
        )),
        "openai-text-line" => {
            outcome.unwrap_err();
        }
        other => panic!("missing outcome for corpus input {other}"),
    }
}
