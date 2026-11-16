#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_openai::error::ProviderError;
use rstest::rstest;

use betterlite_prompts::arguments::json::{Json, Object};

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::tool_calls::repair;
use crate::databricks_chat::response::{
    Reply, ReplyFunction, ReplyToolCall, ResponseInput, transform_response,
};
use crate::databricks_chat::tests::{
    at, at_mut, dump, fill, has, json, len_of, object, set, text_of, unset,
};
use crate::ollama_chat::ids::IdSource;

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        String::from("generated-id")
    }
}

fn call(body: &Json, json_mode: bool) -> Result<Reply, DatabricksChatError> {
    let text = text_of(body);
    transform_response(&ResponseInput {
        body: &text,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: None,
        json_mode,
        ids: &FixedIds,
    })
}

pub(super) fn success(body: &Json, json_mode: bool) -> Json {
    let reply = call(body, json_mode).expect("the response transform succeeds");
    dump(&reply)
}

pub(super) fn failure(body: &Json, json_mode: bool) -> DatabricksChatError {
    call(body, json_mode).expect_err("the response transform fails")
}

fn raw_failure(body: &str) -> DatabricksChatError {
    transform_response(&ResponseInput {
        body,
        status_code: 429,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("3"))]),
        custom_llm_provider: None,
        json_mode: false,
        ids: &FixedIds,
    })
    .expect_err("the body cannot be parsed")
}

pub(super) fn body(message: Json) -> Json {
    body_with(choice(message, json!({})))
}

fn choice(message: Json, extra: Json) -> Json {
    let mut choice = object([
        ("index", json!(0)),
        ("finish_reason", json!("stop")),
        ("message", message),
    ]);
    if let (Json::Object(fields), Json::Object(more)) = (&mut choice, extra) {
        fields.extend(more);
    }
    choice
}

fn body_with(choice: Json) -> Json {
    object([
        ("id", json!("chat-1")),
        ("created", json!(1700)),
        ("model", json!("m")),
        ("choices", Json::List(vec![choice])),
        (
            "usage",
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}),
        ),
    ])
}

pub(super) fn usage_of(reply: &Json) -> Json {
    at(reply, "usage").clone()
}

pub(super) fn message_of(reply: &Json) -> Json {
    at(reply, "choices.0.message").clone()
}

#[test]
fn transforms_a_text_reply_into_the_openai_shape() {
    let output = success(
        &body(json!({"role": "assistant", "content": "hello"})),
        false,
    );
    assert_eq!(
        output,
        json!({
            "id": "chat-1",
            "created": 1700,
            "model": "databricks/m",
            "object": "chat.completion",
            "system_fingerprint": null,
            "choices": [{
                "finish_reason": "stop",
                "index": 0,
                "message": {
                    "content": "hello",
                    "role": "assistant",
                    "tool_calls": null,
                    "function_call": null
                }
            }],
            "usage": {
                "completion_tokens": 2,
                "prompt_tokens": 1,
                "total_tokens": 3,
                "completion_tokens_details": null,
                "prompt_tokens_details": null
            }
        })
    );
}

#[test]
fn a_custom_provider_prefixes_the_model_name() {
    let input = ResponseInput {
        body: r#"{"id": "i", "created": 1, "model": "m", "choices": [], "usage": {}}"#,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: Some("my-provider"),
        json_mode: false,
        ids: &FixedIds,
    };
    let reply = transform_response(&input).expect("the response transform succeeds");
    assert_eq!(reply.model, "my-provider/m");
}

#[test]
fn an_empty_provider_falls_back_to_databricks() {
    let input = ResponseInput {
        body: r#"{"id": "i", "created": 1, "model": "m", "choices": [], "usage": {}}"#,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: Some(""),
        json_mode: false,
        ids: &FixedIds,
    };
    let reply = transform_response(&input).expect("the response transform succeeds");
    assert_eq!(reply.model, "databricks/m");
}

#[test]
fn a_missing_model_becomes_an_empty_name() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    unset(&mut value, "model");
    assert_eq!(*at(&success(&value, false), "model"), json!("databricks/"));
}

#[rstest]
#[case("not json")]
#[case("[]")]
#[case("\"text\"")]
#[case("null")]
fn a_body_that_is_not_a_json_object_is_a_parse_failure(#[case] text: &str) {
    let error = raw_failure(text);
    let DatabricksChatError::Response {
        status_code,
        message,
        headers,
    } = &error
    else {
        panic!("a parse failure is a response error");
    };
    assert_eq!(*status_code, 429);
    assert_eq!(headers.len(), 1);
    assert!(message.starts_with("Unable to get json response - "));
    assert!(message.ends_with(&format!(", Original Response: {text}")));
    assert_eq!(error.status_code(), 429);
}

#[test]
fn a_response_error_keeps_its_headers_through_the_provider_error() {
    let provider = ProviderError::from(raw_failure("not json"));
    assert_eq!(provider.status_code, 429);
    assert_eq!(
        provider.headers,
        Some(vec![(String::from("retry-after"), String::from("3"))])
    );
}

#[rstest]
#[case("id", json!(5))]
#[case("created", json!(null))]
#[case("choices", json!({}))]
#[case("usage", json!([]))]
#[case("usage", json!("x"))]
fn a_broken_top_level_field_is_a_shape_error(#[case] field: &str, #[case] value: Json) {
    let mut body = body(json!({"role": "assistant", "content": "x"}));
    set(&mut body, field, value);
    assert_eq!(failure(&body, false).status_code(), 500);
}

#[rstest]
#[case(json!({"finish_reason": "stop"}), "a Databricks value cannot be read as JSON")]
#[case(json!({"message": {"role": "assistant", "content": "x"}}), "index")]
#[case(json!({"message": {"role": "assistant", "content": "x"}, "index": 0}), "finish_reason")]
#[case(json!({"message": {"role": "assistant"}, "index": 0, "finish_reason": "stop"}), "content")]
fn a_missing_choice_field_is_a_shape_error(#[case] choice: Json, #[case] name: &str) {
    let error = failure(&body_with(choice), false);
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains(name), "{error}");
}

#[rstest]
#[case(json!(null), json!(0))]
#[case(json!(3), json!(3))]
fn the_choice_index_defaults_a_null(#[case] index: Json, #[case] expected: Json) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        fill(json!({"index": "$0"}), "$0", &index),
    );
    assert_eq!(
        *at(&success(&body_with(choice), false), "choices.0.index"),
        expected
    );
}

#[test]
fn finish_reasons_are_mapped_and_keep_the_native_value() {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": "content_filtered"}),
    );
    let output = success(&body_with(choice), false);
    assert_eq!(
        *at(&output, "choices.0.finish_reason"),
        json!("content_filter")
    );
    assert_eq!(
        *at(&output, "choices.0.provider_specific_fields"),
        json!({"native_finish_reason": "content_filtered"})
    );
}

#[rstest]
#[case(json!(null), json!("stop"))]
#[case(json!("length"), json!("length"))]
#[case(json!("tool_calls"), json!("tool_calls"))]
fn a_known_or_null_finish_reason_adds_no_native_value(#[case] raw: Json, #[case] expected: Json) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        fill(json!({"finish_reason": "$0"}), "$0", &raw),
    );
    let output = success(&body_with(choice), false);
    assert_eq!(*at(&output, "choices.0.finish_reason"), expected);
    assert!(!has(at(&output, "choices.0"), "provider_specific_fields"));
}

#[rstest]
#[case(json!(""))]
#[case(json!("weird"))]
fn an_unmapped_finish_reason_defaults_to_stop(#[case] raw: Json) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        fill(json!({"finish_reason": "$0"}), "$0", &raw),
    );
    let output = success(&body_with(choice), false);
    assert_eq!(*at(&output, "choices.0.finish_reason"), json!("stop"));
    assert_eq!(
        *at(&output, "choices.0.provider_specific_fields"),
        fill(json!({"native_finish_reason": "$0"}), "$0", &raw)
    );
}

#[test]
fn a_non_string_finish_reason_is_a_shape_error() {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": 5}),
    );
    let error = failure(&body_with(choice), false);
    assert_eq!(error.status_code(), 500);
}

#[test]
fn text_content_blocks_concatenate() {
    let message = json!({
        "role": "assistant",
        "content": [
            {"type": "text", "text": "a", "zz": 1},
            {"type": "image_url", "image_url": {"url": "u"}},
            {"type": "text", "text": "b"}
        ]
    });
    assert_eq!(
        *at(&message_of(&success(&body(message), false)), "content"),
        json!("ab")
    );
}

#[test]
fn reasoning_blocks_become_reasoning_content_and_thinking_blocks() {
    let message = json!({
        "role": "assistant",
        "content": [
            {"type": "reasoning", "summary": [{"text": "think", "signature": "sig"}, {"text": "more"}]},
            {"type": "text", "text": "answer"}
        ]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "content"), json!("answer"));
    assert_eq!(*at(&output, "reasoning_content"), json!("thinkmore"));
    assert_eq!(
        *at(&output, "thinking_blocks"),
        json!([
            {"type": "thinking", "thinking": "think", "signature": "sig"},
            {"type": "thinking", "thinking": "more", "signature": ""}
        ])
    );
}

#[test]
fn a_null_signature_stays_null() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "reasoning", "summary": [{"text": "t", "signature": null}]}]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "content"), json!(""));
    assert_eq!(
        *at(&output, "thinking_blocks"),
        json!([{"type": "thinking", "thinking": "t", "signature": null}])
    );
}

#[test]
fn a_reasoning_summary_without_text_is_a_shape_error() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "reasoning", "summary": [{"signature": "s"}]}]
    });
    let error = failure(&body(message), false);
    assert_eq!(error.status_code(), 500);
}

#[test]
fn the_blocks_reasoning_wins_over_a_top_level_reasoning_content() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "reasoning", "summary": [{"text": "blk"}]}, {"type": "text", "text": "a"}],
        "reasoning_content": "top"
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "reasoning_content"), json!("blk"));
    assert!(has(&output, "thinking_blocks"));
}

#[test]
fn a_thinking_tag_is_split_without_a_reasoning_key() {
    let message = json!({"role": "assistant", "content": "<think>inside</think>rest"});
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "content"), json!("rest"));
    assert_eq!(*at(&output, "reasoning_content"), json!("inside"));
    assert!(!has(&output, "thinking_blocks"));
}

#[test]
fn a_top_level_reasoning_content_skips_tag_parsing() {
    let message = json!({
        "role": "assistant",
        "content": "<think>inside</think>rest",
        "reasoning_content": "top"
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "content"), json!("<think>inside</think>rest"));
    assert_eq!(*at(&output, "reasoning_content"), json!("top"));
}

#[test]
fn a_null_reasoning_content_key_blocks_tag_parsing() {
    let message = json!({
        "role": "assistant",
        "content": "<think>inside</think>rest",
        "reasoning_content": null
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "content"), json!("<think>inside</think>rest"));
    assert!(!has(&output, "reasoning_content"));
}

#[test]
fn a_reasoning_key_is_used_when_no_reasoning_content_is_set() {
    let message = json!({"role": "assistant", "content": "x", "reasoning": "why"});
    assert_eq!(
        *at(
            &message_of(&success(&body(message), false)),
            "reasoning_content"
        ),
        json!("why")
    );
}

#[rstest]
#[case(json!(5))]
#[case(json!(true))]
#[case(json!({"a": 1}))]
fn a_broken_content_value_is_a_shape_error(#[case] content: Json) {
    let message = fill(
        json!({"role": "assistant", "content": "$0"}),
        "$0",
        &content,
    );
    assert_eq!(failure(&body(message), false).status_code(), 500);
}

#[test]
fn citations_land_in_provider_specific_fields() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "answer", "citations": [{"start": 0, "end": 2}]}]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(
        *at(&output, "provider_specific_fields"),
        json!({"citations": [[{"start": 0, "end": 2, "supported_text": "answer"}]]})
    );
}

#[rstest]
#[case(json!([{"type": "text", "text": "a", "citations": []}]))]
#[case(json!([{"type": "text", "text": "a"}]))]
fn empty_citations_do_not_add_provider_specific_fields(#[case] content: Json) {
    let message = fill(
        json!({"role": "assistant", "content": "$0"}),
        "$0",
        &content,
    );
    let output = message_of(&success(&body(message), false));
    assert!(!has(&output, "provider_specific_fields"));
}

#[test]
fn a_message_without_a_content_key_is_a_shape_error() {
    let error = failure(&body(json!({"role": "assistant"})), false);
    assert_eq!(error.status_code(), 500);
    assert!(matches!(
        error,
        DatabricksChatError::MissingKey { key: "content" }
    ));
}

#[test]
fn tool_calls_are_normalized() {
    let message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [
            {"id": "t1", "function": {"name": "f", "arguments": "{\"a\":1}"}},
            {"function": {"name": "g"}},
            {"id": "t3", "type": "weird", "index": 3, "zz": 1, "function": {"name": "h", "arguments": null, "parameters": {"b": 2}, "extra": true}}
        ]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(
        *at(&output, "tool_calls"),
        json!([
            {"function": {"arguments": "{\"a\":1}", "name": "f"}, "id": "t1", "type": "function"},
            {"function": {"arguments": "", "name": "g"}, "id": "generated-id", "type": "function"},
            {"index": 3, "zz": 1, "function": {"arguments": "{\"b\": 2}", "name": "h"}, "id": "t3", "type": "weird"}
        ])
    );
}

#[test]
fn a_dict_arguments_value_is_encoded() {
    let message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{"id": "t1", "function": {"name": "f", "arguments": {"a": 1}}}]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(
        *at(&output, "tool_calls.0.function.arguments"),
        json!("{\"a\": 1}")
    );
}

#[rstest]
#[case(json!({"id": "t1"}))]
#[case(json!({"id": "t1", "function": null}))]
#[case(json!({"id": "t1", "function": "f"}))]
#[case(json!({"id": 5, "function": {"name": "f"}}))]
#[case(json!({"id": "t1", "type": 5, "function": {"name": "f"}}))]
#[case(json!({"id": "t1", "function": {"name": 5}}))]
#[case(json!({"id": "t1", "function": {"name": "f", "arguments": 5}}))]
#[case(json!({"id": "t1", "function": {"name": "f", "arguments": null, "parameters": "p"}}))]
fn a_broken_tool_call_is_a_decode_error(#[case] tool_call: Json) {
    let value = fill(
        json!({"role": "assistant", "content": null, "tool_calls": ["$0"]}),
        "$0",
        &tool_call,
    );
    assert_decode_error(&body(value), false);
}

// The body is refused as a 500 whose cause is a value outside its declared types
fn assert_decode_error(value: &Json, json_mode: bool) {
    let error = failure(value, json_mode);
    assert_eq!(error.status_code(), 500);
    assert!(matches!(error, DatabricksChatError::Decode(_)), "{error}");
}

#[rstest]
#[case(
    json!({"id": "t1", "type": "custom", "function": {"name": "f"}}),
    json!({"function": {"arguments": "", "name": "f"}, "id": "t1", "type": "custom"})
)]
#[case(
    json!({"id": "t1", "custom": {"name": "f"}, "function": {"name": "f"}}),
    json!({"custom": {"name": "f"}, "function": {"arguments": "", "name": "f"}, "id": "t1", "type": "function"})
)]
fn a_custom_payload_on_a_reply_call_passes_through(
    #[case] tool_call: Json,
    #[case] expected: Json,
) {
    let value = fill(
        json!({"role": "assistant", "content": null, "tool_calls": ["$0"]}),
        "$0",
        &tool_call,
    );
    assert_eq!(
        *at(&message_of(&success(&body(value), false)), "tool_calls"),
        fill(json!(["$0"]), "$0", &expected)
    );
}

#[test]
fn a_null_custom_key_passes_through_as_an_extra() {
    let message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{"id": "t1", "custom": null, "function": {"name": "f", "arguments": "{}"}}]
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "tool_calls.0.function.name"), json!("f"));
    assert_eq!(*at(&output, "tool_calls.0.custom"), Json::Null);
}

#[rstest]
#[case(json!(null))]
#[case(json!([]))]
fn an_empty_tool_calls_value_becomes_null(#[case] tool_calls: Json) {
    let output = message_of(&success(&body(text_message(tool_calls)), false));
    assert_eq!(*at(&output, "tool_calls"), Json::Null);
}

// An assistant message with some text and the given tool_calls value
fn text_message(tool_calls: Json) -> Json {
    object([
        ("role", json!("assistant")),
        ("content", json!("x")),
        ("tool_calls", tool_calls),
    ])
}

#[rstest]
#[case(json!("x"))]
#[case(json!(""))]
#[case(json!(5))]
#[case(json!(true))]
#[case(json!({}))]
#[case(json!({"a": 1}))]
fn a_non_list_tool_calls_value_is_a_shape_error(#[case] tool_calls: Json) {
    assert_eq!(
        failure(&body(text_message(tool_calls)), false).status_code(),
        500
    );
}

fn json_mode_body(tool_calls: Json) -> Json {
    body(object([
        ("role", json!("assistant")),
        ("content", Json::Null),
        ("tool_calls", tool_calls),
    ]))
}

#[test]
fn json_mode_turns_a_json_tool_call_into_content() {
    let tool_calls =
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&output, "choices.0.finish_reason"), json!("stop"));
    assert_eq!(*at(&message_of(&output), "content"), json!("{\"a\":1}"));
    assert_eq!(*at(&message_of(&output), "tool_calls"), json!(null));
}

#[test]
fn json_mode_keeps_a_call_with_another_name() {
    let tool_calls = json!([{"id": "t1", "function": {"name": "f", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert!(len_of(at(&message_of(&output), "tool_calls")).is_some());
}

#[test]
fn json_mode_keeps_two_calls() {
    let tool_calls = json!([
        {"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}},
        {"id": "t2", "function": {"name": "f", "arguments": "x"}}
    ]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert_eq!(len_of(at(&message_of(&output), "tool_calls")), Some(2));
}

#[test]
fn json_mode_without_the_flag_does_not_convert() {
    let tool_calls =
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), false);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
}

#[test]
fn json_mode_with_empty_arguments_does_not_convert() {
    let tool_calls = json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": ""}}]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert!(len_of(at(&message_of(&output), "tool_calls")).is_some());
}

#[test]
fn json_mode_does_not_need_a_finish_reason() {
    let mut value = json_mode_body(
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{}"}}]),
    );
    unset(at_mut(&mut value, "choices.0"), "finish_reason");
    let output = success(&value, true);
    assert_eq!(*at(&output, "choices.0.finish_reason"), json!("stop"));
}

#[test]
fn json_mode_repairs_a_parallel_call_before_converting() {
    let arguments = json!({"tool_uses": [{"recipient_name": "functions.json_tool_call", "parameters": {"a": 1}}]});
    let tool_calls = fill(
        json!([{
            "id": "t1",
            "function": {"name": "multi_tool_use.parallel", "arguments": "$0"}
        }]),
        "$0",
        &text_of(&arguments),
    );
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!("{\"a\": 1}"));
    assert_eq!(*at(&message_of(&output), "tool_calls"), json!(null));
}

#[test]
fn a_parallel_call_with_invalid_json_is_left_alone() {
    let tool_calls = json!([{
        "id": "t1",
        "function": {"name": "multi_tool_use.parallel", "arguments": "not json"}
    }]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert_eq!(*at(&message_of(&output), "tool_calls.0.id"), json!("t1"));
}

#[rstest]
#[case("5")]
#[case("{}")]
#[case("{\"tool_uses\": {}}")]
#[case("{\"tool_uses\": [\"x\"]}")]
#[case("{\"tool_uses\": [{\"parameters\": {}}]}")]
#[case("{\"tool_uses\": [{\"recipient_name\": 5, \"parameters\": {}}]}")]
#[case("{\"tool_uses\": [{\"recipient_name\": \"f\"}]}")]
fn a_broken_parallel_call_is_a_decode_error(#[case] arguments: &str) {
    let function = object([
        ("name", json!("multi_tool_use.parallel")),
        ("arguments", Json::Text(arguments.to_owned())),
    ]);
    let tool_call = object([("id", json!("t1")), ("function", function)]);
    assert_decode_error(&json_mode_body(Json::List(vec![tool_call])), true);
}

fn reply_call(id: &str, name: &str, arguments: &str) -> ReplyToolCall {
    ReplyToolCall {
        extra: Object::new(),
        function: ReplyFunction {
            arguments: arguments.to_owned(),
            name: Some(name.to_owned()),
        },
        id: id.to_owned(),
        kind: String::from("function"),
    }
}

#[test]
fn a_parallel_call_expands_in_place_with_numbered_ids() {
    let arguments = json!({"tool_uses": [
        {"recipient_name": "functions.f", "parameters": {"a": 1}},
        {"recipient_name": "g", "parameters": null}
    ]});
    let calls = [
        reply_call("t0", "first", "{}"),
        reply_call("t1", "multi_tool_use.parallel", &text_of(&arguments)),
        reply_call("t2", "last", "[]"),
    ];
    let repaired = repair(&calls).expect("the repair succeeds");
    assert_eq!(
        repaired,
        vec![
            reply_call("t0", "first", "{}"),
            reply_call("t1_0", "f", "{\"a\": 1}"),
            reply_call("t1_1", "g", "null"),
            reply_call("t2", "last", "[]"),
        ]
    );
}

#[test]
fn a_mixed_list_keeps_the_original_calls_in_the_reply() {
    let arguments =
        json!({"tool_uses": [{"recipient_name": "functions.f", "parameters": {"a": 1}}]});
    let tool_calls = fill(
        json!([
            {"id": "t0", "function": {"name": "json_tool_call", "arguments": "{}"}},
            {"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "$0"}}
        ]),
        "$0",
        &text_of(&arguments),
    );
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert_eq!(*at(&output, "choices.0.finish_reason"), json!("stop"));
    assert_eq!(
        *at(&message_of(&output), "tool_calls.1.function.name"),
        json!("multi_tool_use.parallel")
    );
}

#[test]
fn json_mode_keeps_an_empty_parallel_call_next_to_the_json_tool_call() {
    let tool_calls = json!([
        {"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": []}"}},
        {"id": "t2", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}}
    ]);
    let mut value = json_mode_body(tool_calls);
    set(
        at_mut(&mut value, "choices.0"),
        "finish_reason",
        json!("tool_calls"),
    );
    let output = success(&value, true);
    assert_eq!(*at(&message_of(&output), "content"), json!(null));
    assert_eq!(*at(&output, "choices.0.finish_reason"), json!("tool_calls"));
    assert_eq!(len_of(at(&message_of(&output), "tool_calls")), Some(2));
}

#[test]
fn an_empty_parallel_call_stays_in_the_repaired_list() {
    let calls = [
        reply_call("t1", "json_tool_call", "{\"a\":1}"),
        reply_call("t2", "multi_tool_use.parallel", "{\"tool_uses\": []}"),
    ];
    assert_eq!(repair(&calls).expect("the repair succeeds"), calls.to_vec());
}

#[test]
fn a_sibling_call_with_invalid_json_stops_the_repair() {
    let arguments = json!({"tool_uses": [
        {"recipient_name": "functions.json_tool_call", "parameters": {"a": 1}}
    ]});
    let calls = [
        reply_call("t0", "f", ""),
        reply_call("t1", "multi_tool_use.parallel", &text_of(&arguments)),
    ];
    assert_eq!(repair(&calls).expect("the repair succeeds"), calls.to_vec());
}

#[test]
fn a_broken_parallel_call_after_an_invalid_json_call_is_left_alone() {
    let tool_calls = json!([
        {"id": "t0", "function": {"name": "f", "arguments": "not json"}},
        {"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "5"}}
    ]);
    let output = success(&json_mode_body(tool_calls), true);
    assert_eq!(len_of(at(&message_of(&output), "tool_calls")), Some(2));
}

#[test]
fn a_parallel_call_repair_runs_even_without_json_mode() {
    let tool_calls = json!([{
        "id": "t1",
        "function": {"name": "multi_tool_use.parallel", "arguments": "5"}
    }]);
    assert_eq!(
        failure(&json_mode_body(tool_calls), false).status_code(),
        500
    );
}

#[test]
fn a_message_level_function_call_is_dropped() {
    let message = json!({
        "role": "assistant",
        "content": "x",
        "function_call": {"name": "f", "arguments": "{}"}
    });
    let output = message_of(&success(&body(message), false));
    assert_eq!(*at(&output, "function_call"), json!(null));
}
