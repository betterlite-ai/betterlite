#![cfg(test)]

use betterlite_openai::stream::chunk::Chunk;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use rstest::rstest;

use crate::sagemaker_stream::SagemakerChunk;
use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::project::{
    CHUNK_OBJECT, END_OF_TEXT, Projection, STOP_REASON, project,
};

const MODEL: &str = "decoder-model";

fn projected(text: &str, messages: bool) -> SagemakerChunk {
    match project(MODEL, messages, text).unwrap() {
        Projection::Ready(chunk) => *chunk,
        Projection::Pending => panic!("the fixture is JSON"),
    }
}

fn chat(text: &str) -> Chunk {
    let SagemakerChunk::Chat(chunk) = projected(text, true) else {
        panic!("a messages event yields a chat chunk");
    };
    chunk
}

fn token(text: &str) -> GenericChunk {
    let SagemakerChunk::Token(chunk) = projected(text, false) else {
        panic!("a token event yields a token chunk");
    };
    chunk
}

fn project_error(text: &str, messages: bool) -> SagemakerStreamError {
    let Err(error) = project(MODEL, messages, text) else {
        panic!("the fixture is refused");
    };
    error
}

#[test]
fn the_object_is_forced_and_the_model_defaults_to_the_decoder_one() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": []}"#);

    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.id, "1");
    assert_eq!(chunk.created, 2);
    assert_eq!(chunk.choices.len(), 0);
    assert!(chunk.usage.is_none());
}

#[test]
fn a_chunk_object_is_replaced_with_the_chunk_one() {
    let chunk = chat(
        r#"{"id": "1", "object": "chat.completion", "created": 2, "model": "own", "choices": []}"#,
    );

    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.model, "own");
}

#[test]
fn a_null_model_takes_the_decoder_model() {
    let chunk = chat(r#"{"id": "1", "created": 2, "model": null, "choices": []}"#);

    assert_eq!(chunk.model, MODEL);
}

#[test]
fn a_messages_chunk_keeps_its_choices() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hi"}, "finish_reason": null}]}"#,
    );

    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(chunk.choices[0].index, 0);
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunk.choices[0].finish_reason, None);
}

// every field the chunk declares reaches the client, not only the ones earlier cases sampled
#[test]
fn a_messages_chunk_keeps_its_usage_and_choice_details() {
    let chunk = chat(
        r#"{"id": "full", "created": 4, "system_fingerprint": "fp_1", "service_tier": "default", "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}, "choices": [{"index": 0, "delta": {"refusal": "no", "function_call": {"name": "g", "arguments": "{\"a\": 1}"}}, "logprobs": {"content": [{"token": "hi", "logprob": -0.5, "bytes": [104, 105], "top_logprobs": [{"token": "hi", "logprob": -0.25, "bytes": null}]}]}, "finish_reason": "stop"}]}"#,
    );
    let choice = &chunk.choices[0];
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 1);
    assert_eq!(usage.total_tokens, 4);
    assert_eq!(chunk.system_fingerprint.as_deref(), Some("fp_1"));
    assert_eq!(chunk.service_tier.as_deref(), Some("default"));
    assert_eq!(choice.delta.refusal.as_deref(), Some("no"));
    let function = choice.delta.function_call.as_ref().unwrap();
    assert_eq!(function.name.as_deref(), Some("g"));
    assert_eq!(function.arguments, r#"{"a": 1}"#);
    let logprobs = choice.logprobs.as_ref().unwrap();
    let entries = logprobs.content.as_ref().unwrap();
    assert_eq!(entries[0].token, "hi");
    assert_eq!(entries[0].logprob.to_string(), "-0.5");
    assert_eq!(entries[0].bytes, Some(vec![104, 105]));
    assert_eq!(entries[0].top_logprobs[0].token, "hi");
    assert_eq!(entries[0].top_logprobs[0].bytes, None);
    assert!(logprobs.refusal.is_none());
}

#[test]
fn a_chunk_without_choices_is_refused() {
    let error = project_error(r#"{"id": "1", "created": 2}"#, true);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
    assert_eq!(
        error.to_string(),
        "the SageMaker stream chunk does not have the declared shape"
    );
}

// the OpenAI chunk models set extra="allow", so an extra key is read past instead of failing the chunk
#[test]
fn an_unknown_chunk_field_is_ignored() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": [], "extra": 1}"#);

    assert_eq!(chunk.id, "1");
    assert_eq!(chunk.choices.len(), 0);
}

#[test]
fn a_non_object_messages_chunk_is_refused() {
    let error = project_error("[1]", true);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

// serde fills a struct from a positional array, so the object check must come before either chunk shape
#[rstest]
#[case::token(false, "[null, 0]")]
#[case::messages(true, r#"["1", 2, null, [], null, null, null]"#)]
fn a_positional_array_is_refused(#[case] messages: bool, #[case] text: &str) {
    let error = project_error(text, messages);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_partial_object_is_pending() {
    assert!(matches!(
        project(MODEL, true, r#"{"id": "1", "choices": ["#),
        Ok(Projection::Pending)
    ));
}

// StreamingChoices.__init__ maps an alias through map_finish_reason before the chunk reaches a client
#[test]
fn an_alias_finish_reason_maps_to_stop() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": [{"finish_reason": "eos_token"}]}"#);

    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some(STOP_REASON));
}

// StreamingChoices defaults index to 0 and delta to an empty Delta when the choice omits them
#[test]
fn a_choice_without_index_or_delta_takes_the_defaults() {
    let chunk = chat(r#"{"id": "1", "created": 2, "choices": [{}]}"#);
    let choice = &chunk.choices[0];

    assert_eq!(choice.index, 0);
    assert!(choice.delta.role.is_none());
    assert!(choice.delta.content.is_none());
    assert!(choice.delta.tool_calls.is_none());
}

// StreamingChoices stores None when the stop reason is falsy, so an empty one never reaches a client
#[test]
fn a_null_delta_and_an_empty_finish_reason_take_the_defaults() {
    let chunk =
        chat(r#"{"id": "1", "created": 2, "choices": [{"delta": null, "finish_reason": ""}]}"#);

    assert!(chunk.choices[0].delta.content.is_none());
    assert_eq!(chunk.choices[0].finish_reason, None);
}

// Delta.__init__ numbers a tool call without an index and types it function
#[test]
fn a_tool_call_without_index_or_type_takes_the_defaults() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].index, 0);
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(
        calls[0].function.as_ref().unwrap().name.as_deref(),
        Some("f")
    );
}

// the counter advances only for a call that had no index, so an explicit 7 is followed by 0
#[test]
fn an_explicit_tool_call_index_leaves_the_counter_alone() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"index": 7, "type": "custom", "function": {}}, {}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].index, 7);
    assert_eq!(calls[0].kind.as_deref(), Some("custom"));
    assert_eq!(calls[1].index, 0);
    assert_eq!(calls[1].kind.as_deref(), Some("function"));
}

// each unindexed call takes the next counter value, so the second one is 1
#[test]
fn two_tool_calls_without_indexes_are_numbered_in_order() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {}}, {"function": {}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].index, 0);
    assert_eq!(calls[1].index, 1);
}

// Function.__init__ dumps a dict argument with Python's separators, so the call carries a JSON string
#[test]
fn a_dict_tool_call_arguments_is_dumped_like_python() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "arguments": {"a": 1, "b": "x"}}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(
        calls[0].function.as_ref().unwrap().arguments,
        r#"{"a": 1, "b": "x"}"#
    );
}

// Function.__init__ reads a null argument as the empty string, not as the text "null"
#[test]
fn a_null_tool_call_arguments_is_empty() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "arguments": null}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].function.as_ref().unwrap().arguments, "");
}

#[test]
fn a_text_tool_call_arguments_is_used_as_given() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "arguments": "{\"a\": 1}"}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].function.as_ref().unwrap().arguments, r#"{"a": 1}"#);
}

// Function.__init__ dumps a dict parameters when the call omits arguments, and ignores a non-dict one
#[test]
fn a_parameters_dict_stands_in_for_missing_arguments() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "parameters": {"a": 1}}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].function.as_ref().unwrap().arguments, r#"{"a": 1}"#);
}

#[test]
fn a_non_dict_parameters_reads_as_empty_arguments() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"name": "f", "parameters": [1]}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].function.as_ref().unwrap().arguments, "");
}

#[test]
fn arguments_win_over_parameters() {
    let chunk = chat(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"arguments": "{}", "parameters": {"a": 1}}}]}}]}"#,
    );
    let calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();

    assert_eq!(calls[0].function.as_ref().unwrap().arguments, "{}");
}

#[test]
fn a_number_tool_call_arguments_is_refused() {
    let error = project_error(
        r#"{"id": "1", "created": 2, "choices": [{"delta": {"tool_calls": [{"function": {"arguments": 5}}]}}]}"#,
        true,
    );

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_missing_token_and_index_read_as_empty_and_zero() {
    let chunk = token("{}");

    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(0));
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
    assert!(chunk.usage.is_none());
    assert!(chunk.tool_use.is_none());
    assert!(chunk.provider_specific_fields.is_none());
}

#[test]
fn a_null_token_reads_as_an_empty_one() {
    let chunk = token(r#"{"token": null}"#);

    assert_eq!(chunk.text, "");
}

#[test]
fn a_null_text_reads_as_empty() {
    let chunk = token(r#"{"token": {"text": null}}"#);

    assert_eq!(chunk.text, "");
    assert!(!chunk.is_finished);
}

#[test]
fn a_text_token_is_used_as_given() {
    let chunk = token(r#"{"token": {"text": "hello world"}, "index": 7}"#);

    assert_eq!(chunk.text, "hello world");
    assert_eq!(chunk.index, Some(7));
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
}

#[test]
fn the_end_of_text_token_finishes_with_stop() {
    let fixture = format!(r#"{{"token": {{"text": "{END_OF_TEXT}"}}, "index": 2}}"#);

    let chunk = token(&fixture);
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(2));
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, STOP_REASON);
    assert!(chunk.usage.is_none());
}

#[test]
fn an_index_is_used_as_given_negative_included() {
    let chunk = token(r#"{"index": -3}"#);

    assert_eq!(chunk.index, Some(-3));
}

#[test]
fn a_zero_index_stays_zero() {
    let chunk = token(r#"{"token": {"text": "x"}, "index": 0}"#);

    assert_eq!(chunk.index, Some(0));
    assert_eq!(chunk.text, "x");
}

#[test]
fn a_null_index_is_zero() {
    let chunk = token(r#"{"index": null}"#);

    assert_eq!(chunk.index, Some(0));
}

#[test]
fn a_text_index_is_refused() {
    let error = project_error(r#"{"index": "first"}"#, false);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_float_index_is_refused() {
    let error = project_error(r#"{"index": 1.5}"#, false);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_list_token_is_refused() {
    let error = project_error(r#"{"token": []}"#, false);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_number_text_is_refused() {
    let error = project_error(r#"{"token": {"text": 5}}"#, false);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}

#[test]
fn a_non_object_token_is_refused() {
    let error = project_error("[1]", false);

    assert!(matches!(error, SagemakerStreamError::Json(_)));
}
