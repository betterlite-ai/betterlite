#![cfg(test)]

use betterlite_prompts::arguments::json::Json as PromptJson;
use rstest::rstest;
use serde_json::{Value, json};

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::stream::{ChunkParser, StreamChunk};
use crate::ollama_chat::ids::IdSource;

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        String::from("generated-id")
    }
}

fn parse_with(body: &Value, json_mode: bool) -> Result<StreamChunk, DatabricksChatError> {
    let chunk: PromptJson =
        serde_json::from_value(body.clone()).expect("the fixture chunk is JSON");
    ChunkParser::new(json_mode).parse(&chunk, &FixedIds)
}

fn success_with(body: &Value, json_mode: bool) -> Value {
    let chunk = parse_with(body, json_mode).expect("the chunk parses");
    serde_json::to_value(&chunk).expect("the chunk serializes")
}

fn success(body: &Value) -> Value {
    success_with(body, false)
}

fn failure(body: &Value) -> DatabricksChatError {
    parse_with(body, false).expect_err("the chunk does not parse")
}

fn body(delta: Value) -> Value {
    chunk_with(json!({"index": 0, "delta": delta}), json!({}))
}

fn chunk_with(choice: Value, extra: Value) -> Value {
    let mut chunk = json!({"id": "chat-1", "created": 1700, "model": "m", "choices": [choice]});
    let fields = chunk.as_object_mut().expect("the chunk is an object");
    for (name, value) in extra.as_object().expect("the extra fields are an object") {
        fields.insert(name.clone(), value.clone());
    }
    chunk
}

fn delta_of(chunk: &Value) -> Value {
    chunk["choices"][0]["delta"].clone()
}

const OPENAI_DELTA: [&str; 5] = ["content", "role", "function_call", "tool_calls", "audio"];

#[test]
fn a_text_chunk_carries_the_openai_chunk_shape() {
    assert_eq!(
        success(&body(json!({"content": "hello"}))),
        json!({
            "id": "chat-1",
            "created": 1700,
            "model": "m",
            "object": "chat.completion.chunk",
            "choices": [{
                "finish_reason": null,
                "index": 0,
                "delta": {
                    "content": "hello",
                    "role": null,
                    "function_call": null,
                    "tool_calls": null,
                    "audio": null
                },
                "logprobs": null
            }],
            "provider_specific_fields": null,
            "usage": null
        })
    );
}

#[test]
fn an_empty_delta_dumps_the_five_openai_keys() {
    let delta = delta_of(&success(&body(json!({}))));
    let fields = delta.as_object().expect("the delta is an object");
    assert_eq!(fields.len(), OPENAI_DELTA.len());
    for name in OPENAI_DELTA {
        assert_eq!(fields[name], Value::Null, "{name} is null");
    }
}

#[test]
fn a_role_and_top_level_reasoning_are_kept() {
    let delta = delta_of(&success(&body(
        json!({"role": "assistant", "reasoning_content": "because"}),
    )));
    assert_eq!(delta["role"], json!("assistant"));
    assert_eq!(delta["reasoning_content"], json!("because"));
}

#[test]
fn a_reasoning_key_becomes_the_reasoning_content() {
    let delta = delta_of(&success(&body(json!({"content": "x", "reasoning": "r"}))));
    assert_eq!(delta["reasoning_content"], json!("r"));
    assert!(delta.get("reasoning").is_none());
}

#[test]
fn a_reasoning_key_stays_an_extra_when_reasoning_content_is_set() {
    let delta = delta_of(&success(&body(
        json!({"content": "x", "reasoning": "r", "reasoning_content": "rc"}),
    )));
    assert_eq!(delta["reasoning_content"], json!("rc"));
    assert_eq!(delta["reasoning"], json!("r"));
}

#[rstest]
#[case(json!(""), json!(null))]
#[case(json!(null), json!(null))]
#[case(json!(false), json!(null))]
#[case(json!(0), json!(null))]
#[case(json!([]), json!(null))]
#[case(json!({}), json!(null))]
#[case(json!("length"), json!("length"))]
#[case(json!("tool_use"), json!("tool_calls"))]
#[case(json!("weird"), json!("stop"))]
#[case(json!(true), json!("stop"))]
#[case(json!(5), json!("stop"))]
fn the_finish_reason_maps_like_the_openai_one(#[case] raw: Value, #[case] expected: Value) {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}, "finish_reason": raw}),
        json!({}),
    );
    assert_eq!(success(&chunk)["choices"][0]["finish_reason"], expected);
}

#[rstest]
#[case(json!([1]))]
#[case(json!({"a": 1}))]
fn a_non_scalar_finish_reason_is_a_shape_error(#[case] raw: Value) {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}, "finish_reason": raw}),
        json!({}),
    );
    assert_eq!(failure(&chunk).status_code(), 500);
}

#[rstest]
#[case(json!({"index": 0, "delta": {"content": "x"}}), json!(0))]
#[case(json!({"index": "3", "delta": {"content": "x"}}), json!("3"))]
#[case(json!({"index": null, "delta": {"content": "x"}}), json!(null))]
fn the_choice_index_passes_through(#[case] choice: Value, #[case] expected: Value) {
    assert_eq!(
        success(&chunk_with(choice, json!({})))["choices"][0]["index"],
        expected
    );
}

#[test]
fn a_missing_choices_key_is_the_databricks_400() {
    let error = failure(&json!({"id": "i", "created": 1, "model": "m"}));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("KeyError: 'choices'"));
}

#[rstest]
#[case("id")]
#[case("created")]
#[case("model")]
fn a_missing_chunk_key_is_the_databricks_400(#[case] name: &str) {
    let mut chunk = json!({"id": "i", "created": 1, "model": "m", "choices": []});
    chunk
        .as_object_mut()
        .expect("the chunk is an object")
        .remove(name);
    let error = failure(&chunk);
    assert_eq!(error.status_code(), 400);
    assert!(
        error.to_string().contains(&format!("KeyError: '{name}'")),
        "{error}"
    );
}

#[test]
fn a_missing_delta_key_is_the_databricks_400() {
    let error = failure(&chunk_with(json!({"index": 0}), json!({})));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("KeyError: 'delta'"));
}

#[test]
fn a_null_id_is_generated_and_a_number_is_a_shape_error() {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"id": null}),
    );
    assert_eq!(success(&chunk)["id"], json!("generated-id"));
    let mut broken = json!({"id": 5, "created": 1, "model": "m", "choices": []});
    assert_eq!(failure(&broken).status_code(), 500);
    broken["id"] = json!("x");
}

#[test]
fn a_null_model_stays_null() {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"model": null}),
    );
    assert_eq!(success(&chunk)["model"], Value::Null);
}

#[rstest]
#[case(json!("1700"), json!(1700))]
#[case(json!(1700), json!(1700))]
fn created_accepts_an_integer_or_its_string(#[case] raw: Value, #[case] expected: Value) {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"created": raw}),
    );
    assert_eq!(success(&chunk)["created"], expected);
}

#[rstest]
#[case(json!(1.5))]
#[case(json!("1.5"))]
#[case(json!(true))]
#[case(json!(null))]
fn a_created_that_is_not_an_integer_is_an_error(#[case] raw: Value) {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"created": raw}),
    );
    assert_eq!(failure(&chunk).status_code(), 500);
}

#[test]
fn usage_is_normalized_when_it_is_an_object() {
    let chunk = body(json!({"content": "x"}));
    let chunk = chunk_with(
        chunk["choices"][0].clone(),
        json!({
            "usage": {"prompt_tokens": 1, "completion_tokens": 4, "total_tokens": 5, "reasoning_tokens": 3, "zz": 1}
        }),
    );
    let usage = success(&chunk)["usage"].clone();
    assert_eq!(usage["prompt_tokens"], json!(1));
    assert_eq!(usage["completion_tokens"], json!(4));
    assert_eq!(usage["total_tokens"], json!(5));
    assert_eq!(
        usage["completion_tokens_details"],
        json!({"reasoning_tokens": 3, "text_tokens": 1})
    );
    assert_eq!(usage["prompt_tokens_details"], Value::Null);
    assert_eq!(usage["zz"], json!(1));
}

#[test]
fn a_non_object_usage_passes_through() {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"usage": "x"}),
    );
    assert_eq!(success(&chunk)["usage"], json!("x"));
}

#[rstest]
#[case(json!("scale"), Some(json!("scale")))]
#[case(json!(""), None)]
#[case(json!(5), None)]
fn service_tier_appears_only_when_it_is_a_non_empty_string(
    #[case] raw: Value,
    #[case] expected: Option<Value>,
) {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}}),
        json!({"service_tier": raw}),
    );
    let chunk = success(&chunk);
    match expected {
        Some(value) => assert_eq!(chunk["service_tier"], value),
        None => assert!(chunk.get("service_tier").is_none()),
    }
}

#[test]
fn content_blocks_split_into_text_reasoning_and_a_citation() {
    let delta = delta_of(&success(&body(json!({
        "content": [
            {"type": "text", "text": "a", "citations": [{"start": 0}, {"start": 1}]},
            {"type": "reasoning", "summary": [{"text": "r", "signature": "s"}]},
            {"type": "text", "text": "b"}
        ]
    }))));
    assert_eq!(delta["content"], json!("ab"));
    assert_eq!(delta["reasoning_content"], json!("r"));
    assert_eq!(
        delta["thinking_blocks"],
        json!([{"type": "thinking", "thinking": "r", "signature": "s"}])
    );
    assert_eq!(
        delta["provider_specific_fields"],
        json!({"citation": {"start": 0}})
    );
}

#[test]
fn a_citation_merges_into_an_existing_provider_specific_fields() {
    let delta = delta_of(&success(&body(json!({
        "content": [{"type": "text", "text": "a", "citations": [{"start": 0}]}],
        "provider_specific_fields": {"zz": 1}
    }))));
    assert_eq!(
        delta["provider_specific_fields"],
        json!({"zz": 1, "citation": {"start": 0}})
    );
}

#[test]
fn an_empty_citation_list_is_ignored() {
    let delta = delta_of(&success(&body(json!({
        "content": [{"type": "text", "text": "a", "citations": []}]
    }))));
    assert!(delta.get("provider_specific_fields").is_none());
    assert_eq!(delta["content"], json!("a"));
}

#[test]
fn images_appear_only_when_non_empty_and_annotations_pass_through() {
    let empty = delta_of(&success(&body(
        json!({"content": "x", "images": [], "annotations": [{"a": 1}], "reasoning_items": [{"b": 2}]}),
    )));
    assert!(empty.get("images").is_none());
    assert_eq!(empty["annotations"], json!([{"a": 1}]));
    assert_eq!(empty["reasoning_items"], json!([{"b": 2}]));
    let filled = delta_of(&success(&body(
        json!({"content": "x", "images": [{"a": 1}]}),
    )));
    assert_eq!(filled["images"], json!([{"a": 1}]));
}

#[test]
fn the_delta_function_call_keeps_its_extras() {
    let delta = delta_of(&success(&body(
        json!({"function_call": {"name": "f", "arguments": "x", "zz": 1}}),
    )));
    assert_eq!(
        delta["function_call"],
        json!({"arguments": "x", "name": "f", "zz": 1})
    );
}

#[test]
fn a_null_reasoning_and_function_call_are_dropped() {
    let delta = delta_of(&success(&body(
        json!({"content": "x", "reasoning": null, "function_call": null}),
    )));
    assert!(delta.get("reasoning").is_none());
    assert!(delta.get("reasoning_content").is_none());
    assert_eq!(delta["function_call"], json!(null));
}

#[test]
fn a_function_call_without_arguments_is_a_shape_error() {
    let error = failure(&body(json!({"function_call": {"name": "f"}})));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn tool_call_indices_are_assigned_in_order() {
    let delta = delta_of(&success(&body(json!({
        "tool_calls": [
            {"function": {"name": "f", "arguments": "x"}},
            {"index": 3, "function": {"name": "g", "arguments": "y"}},
            {"function": {"name": "h", "arguments": "z"}}
        ]
    }))));
    let calls = delta["tool_calls"]
        .as_array()
        .expect("the calls are a list");
    assert_eq!(calls[0]["index"], json!(0));
    assert_eq!(calls[1]["index"], json!(3));
    assert_eq!(calls[2]["index"], json!(1));
}

#[test]
fn a_tool_call_defaults_its_type_and_dumps_a_null_id() {
    let delta = delta_of(&success(&body(json!({
        "tool_calls": [{"index": 0, "id": null, "function": {"name": "f", "arguments": "x", "zz": 1}, "zz": 2}]
    }))));
    assert_eq!(
        delta["tool_calls"],
        json!([{
            "id": null,
            "function": {"arguments": "x", "name": "f"},
            "type": "function",
            "index": 0,
            "zz": 2
        }])
    );
}

#[rstest]
#[case(json!([]), json!([]))]
#[case(json!({}), json!({}))]
#[case(json!(""), json!(""))]
#[case(json!(null), json!(null))]
fn a_falsy_tool_calls_value_passes_through(#[case] raw: Value, #[case] expected: Value) {
    let delta = delta_of(&success(&body(json!({"content": "x", "tool_calls": raw}))));
    assert_eq!(delta["tool_calls"], expected);
}

#[test]
fn a_non_empty_tool_calls_object_is_a_shape_error() {
    let error = failure(&body(json!({"tool_calls": {"a": 1}})));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_custom_tool_call_passes_through() {
    let delta = delta_of(&success(&body(json!({
        "tool_calls": [{"index": null, "type": "custom", "custom": {"name": "f", "zz": 1}, "function": null, "id": null, "zz": 2}]
    }))));
    assert_eq!(
        delta["tool_calls"],
        json!([{
            "id": null,
            "type": "custom",
            "custom": {"name": "f", "input": null, "zz": 1},
            "index": 0,
            "zz": 2
        }])
    );
}

#[test]
fn a_custom_tool_call_without_a_type_passes_through() {
    let delta = delta_of(&success(&body(json!({
        "tool_calls": [{"custom": {"name": "f"}}]
    }))));
    assert_eq!(
        delta["tool_calls"],
        json!([{
            "id": null,
            "type": null,
            "custom": {"name": "f", "input": null},
            "index": 0
        }])
    );
}

#[test]
fn a_custom_tool_call_without_its_payload_is_a_shape_error() {
    let error = failure(&body(
        json!({"tool_calls": [{"type": "custom", "index": 0}]}),
    ));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn empty_arguments_are_cleared_outside_json_mode() {
    let delta = delta_of(&success(&body(json!({
        "tool_calls": [{"index": 0, "id": "a", "function": {"name": "f", "arguments": "{}"}}]
    }))));
    assert_eq!(delta["tool_calls"][0]["function"]["arguments"], json!(""));
}

#[test]
fn json_mode_turns_a_json_tool_call_into_content() {
    let chunk = body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{\"a\": 1}"}}]
    }));
    let delta = delta_of(&success_with(&chunk, true));
    assert_eq!(delta["content"], json!("{\"a\":1}"));
    assert_eq!(delta["tool_calls"], Value::Null);
}

#[test]
fn json_mode_unwraps_the_values_key() {
    let chunk = body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{\"values\": {\"a\": 1}}"}}]
    }));
    assert_eq!(
        delta_of(&success_with(&chunk, true))["content"],
        json!("{\"a\":1}")
    );
}

#[test]
fn json_mode_keeps_an_invalid_json_string_as_content() {
    let chunk = body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{not json"}}]
    }));
    assert_eq!(
        delta_of(&success_with(&chunk, true))["content"],
        json!("{not json")
    );
}

#[test]
fn json_mode_turns_empty_object_arguments_into_empty_content() {
    let chunk = body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{}"}}]
    }));
    assert_eq!(delta_of(&success_with(&chunk, true))["content"], json!(""));
}

#[test]
fn json_mode_leaves_another_function_name_alone() {
    let chunk = body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "other", "arguments": "{\"a\": 1}"}}]
    }));
    let delta = delta_of(&success_with(&chunk, true));
    assert!(delta.get("content").is_none() || delta["content"].is_null());
    assert_eq!(delta["tool_calls"][0]["function"]["name"], json!("other"));
}

#[test]
fn json_mode_remembers_the_function_name_across_chunks() {
    let first: PromptJson = serde_json::from_value(body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{\"a\":"}}]
    })))
    .expect("the fixture chunk is JSON");
    let second: PromptJson = serde_json::from_value(body(json!({
        "tool_calls": [{"index": 0, "function": {"arguments": "{\"b\": 2}"}}]
    })))
    .expect("the fixture chunk is JSON");
    let mut parser = ChunkParser::new(true);
    parser
        .parse(&first, &FixedIds)
        .expect("the first chunk parses");
    let chunk = parser
        .parse(&second, &FixedIds)
        .expect("the second chunk parses");
    let value = serde_json::to_value(&chunk).expect("the chunk serializes");
    assert_eq!(delta_of(&value)["content"], json!("{\"b\":2}"));
}

#[test]
fn json_mode_needs_the_function_key_even_for_a_remembered_name() {
    let first: PromptJson = serde_json::from_value(body(json!({
        "tool_calls": [{"index": 0, "function": {"name": "json_tool_call", "arguments": "{}"}}]
    })))
    .expect("the fixture chunk is JSON");
    let second: PromptJson = serde_json::from_value(body(json!({
        "tool_calls": [{"index": 0}]
    })))
    .expect("the fixture chunk is JSON");
    let mut parser = ChunkParser::new(true);
    parser
        .parse(&first, &FixedIds)
        .expect("the first chunk parses");
    let error = parser
        .parse(&second, &FixedIds)
        .expect_err("the second chunk is missing its function");
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("KeyError: 'function'"));
}

#[test]
fn a_choice_extra_is_kept_and_its_message_is_dropped() {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}, "zz": 1, "message": {"content": "y"}}),
        json!({}),
    );
    let choice = success(&chunk)["choices"][0].clone();
    assert_eq!(choice["zz"], json!(1));
    assert!(choice.get("message").is_none());
}

#[test]
fn a_logprobs_value_passes_through() {
    let chunk = chunk_with(
        json!({"index": 0, "delta": {"content": "x"}, "logprobs": {"content": []}}),
        json!({}),
    );
    assert_eq!(
        success(&chunk)["choices"][0]["logprobs"],
        json!({"content": []})
    );
}

#[test]
fn a_delta_that_is_not_an_object_is_a_shape_error() {
    let error = failure(&chunk_with(json!({"index": 0, "delta": null}), json!({})));
    assert_eq!(error.status_code(), 500);
}
