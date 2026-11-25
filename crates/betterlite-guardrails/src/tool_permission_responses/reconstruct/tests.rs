#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::tool_permission::types::{mapping_value, object_list};
use crate::tool_permission_responses::tests::support::object;

use super::reconstruct_chunk;

#[test]
fn reconstruct_chunk_builds_one_streaming_chunk_from_a_response() {
    let response = object(
        r#"{"id":"chatcmpl-1","created":123,"model":"gpt","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}"#,
    );
    let chunk = reconstruct_chunk(&response);
    assert_eq!(
        chunk.get("id"),
        Some(&GuardrailValue::Text("chatcmpl-1".to_owned()))
    );
    assert_eq!(
        chunk.get("object"),
        Some(&GuardrailValue::Text("chat.completion.chunk".to_owned()))
    );
    assert_eq!(
        chunk.get("created"),
        Some(&GuardrailValue::SignedInteger(123))
    );
    let choices = chunk.get("choices").and_then(object_list).expect("choices");
    assert_eq!(choices.len(), 1);
    let choice = choices.first().expect("choice");
    assert_eq!(
        mapping_value(choice, "index"),
        Some(&GuardrailValue::SignedInteger(0))
    );
    assert_eq!(
        mapping_value(choice, "finish_reason"),
        Some(&GuardrailValue::Text("tool_calls".to_owned()))
    );
    assert_eq!(
        mapping_value(choice, "delta"),
        Some(&GuardrailValue::Object(object(
            r#"{"role":"assistant","content":"hi"}"#
        )))
    );
    assert!(mapping_value(&GuardrailValue::Object(chunk), "usage").is_some());
}

#[test]
fn reconstruct_chunk_keeps_a_choice_logprobs() {
    let response = object(
        r#"{"choices":[{"index":0,"message":{"content":"hi"},"finish_reason":"stop","logprobs":{"content":[{"token":"hi","logprob":-0.1}]}}]}"#,
    );
    let chunk = reconstruct_chunk(&response);
    let choice = chunk
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        mapping_value(choice, "logprobs"),
        Some(&object(r#"{"content":[{"token":"hi","logprob":-0.1}]}"#))
    );
}

#[test]
fn reconstruct_chunk_leaves_a_null_choice_logprobs_out() {
    let response = object(r#"{"choices":[{"message":{"content":"hi"},"logprobs":null}]}"#);
    let chunk = reconstruct_chunk(&response);
    let choice = chunk
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert!(mapping_value(choice, "logprobs").is_none());
}

#[test]
fn reconstruct_chunk_defaults_missing_choice_fields_to_null() {
    let response = object(r#"{"choices":[{"message":{"content":"hi"}}]}"#);
    let chunk = reconstruct_chunk(&response);
    let choice = chunk
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(mapping_value(choice, "index"), Some(&GuardrailValue::Null));
    assert_eq!(
        mapping_value(choice, "finish_reason"),
        Some(&GuardrailValue::Null)
    );
    assert!(!chunk.contains_key("id"));
    assert!(!chunk.contains_key("usage"));
}

#[test]
fn reconstruct_chunk_has_no_choices_without_a_choices_list() {
    let chunk = reconstruct_chunk(&object(r#"{"id":"x"}"#));
    assert_eq!(
        chunk.get("choices"),
        Some(&GuardrailValue::Array(Vec::new()))
    );
}

#[test]
fn reconstruct_chunk_keeps_a_null_choice_delta() {
    let response = object(r#"{"choices":[7]}"#);
    let chunk = reconstruct_chunk(&response);
    let choice = chunk
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(mapping_value(choice, "delta"), Some(&GuardrailValue::Null));
}
