#![cfg(test)]

use serde_json::from_str;

use crate::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::stream_chunk_shaping::records::{GenericChunk, JsonValue};

fn state(model: &str) -> DecodeState {
    DecodeState {
        model: model.to_owned(),
        kind: ProviderKind::Builtin,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: None,
        options: RequestOptions {
            send_stream_usage: false,
            is_function_call: false,
        },
        repeated_chunk_limit: 100,
    }
}

fn json(source: &str) -> JsonValue {
    from_str(source).expect("the test literal is valid JSON")
}

fn generic_chunk(source: &str) -> GenericChunk {
    from_str(source).expect("the test literal is a generic chunk")
}

#[test]
fn generic_text_carries_the_generic_chunk_fields() {
    let mut generic_state = state("gpt-4");
    let chunk = ProviderChunk::Generic(Box::new(generic_chunk(
        r#"{"text": "hi", "is_finished": false, "finish_reason": "", "usage": null}"#,
    )));
    let decoded = decode_provider_chunk(Some("my-custom"), &chunk, &mut generic_state)
        .expect("a generic chunk decodes");
    assert_eq!(decoded.text, "hi");
    assert!(!decoded.is_finished);
}

#[test]
fn generic_json_chunk_takes_the_generic_branch_for_a_builtin_provider() {
    let mut generic_state = state("gpt-4");
    let chunk = ProviderChunk::Json(Box::new(json(
        r#"{"text": "hello", "is_finished": true, "finish_reason": "stop", "usage": null}"#,
    )));
    let decoded = decode_provider_chunk(Some("openai"), &chunk, &mut generic_state)
        .expect("a generic shaped object decodes");
    assert_eq!(decoded.text, "hello");
    assert_eq!(
        generic_state.received_finish_reason.as_deref(),
        Some("stop")
    );
}
