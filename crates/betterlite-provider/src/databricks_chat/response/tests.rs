#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_openai::error::ProviderError;
use rstest::rstest;
use serde_json::{Value, json};

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::reply::Reply;
use crate::databricks_chat::response::{ResponseInput, transform_response};
use crate::ollama_chat::ids::IdSource;

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        String::from("generated-id")
    }
}

fn call(body: &Value, json_mode: Option<bool>) -> Result<Reply, DatabricksChatError> {
    let text = serde_json::to_string(body).expect("the fixture body encodes");
    transform_response(ResponseInput {
        body: &text,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: None,
        json_mode,
        ids: &FixedIds,
    })
}

fn success(body: &Value, json_mode: Option<bool>) -> Value {
    let reply = call(body, json_mode).expect("the response transform succeeds");
    serde_json::to_value(&reply).expect("the reply serializes")
}

fn failure(body: &Value, json_mode: Option<bool>) -> DatabricksChatError {
    call(body, json_mode).expect_err("the response transform fails")
}

fn raw_failure(body: &str) -> DatabricksChatError {
    transform_response(ResponseInput {
        body,
        status_code: 429,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("3"))]),
        custom_llm_provider: None,
        json_mode: None,
        ids: &FixedIds,
    })
    .expect_err("the body cannot be parsed")
}

fn body(message: Value) -> Value {
    body_with(choice(message, json!({})))
}

fn choice(message: Value, extra: Value) -> Value {
    let mut choice = json!({"index": 0, "finish_reason": "stop", "message": message});
    let fields = choice.as_object_mut().expect("the choice is an object");
    for (name, value) in extra.as_object().expect("the extra fields are an object") {
        fields.insert(name.clone(), value.clone());
    }
    choice
}

fn body_with(choice: Value) -> Value {
    json!({
        "id": "chat-1",
        "created": 1700,
        "model": "m",
        "choices": [choice],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    })
}

fn usage_of(reply: &Value) -> Value {
    reply.get("usage").expect("the reply carries usage").clone()
}

fn message_of(reply: &Value) -> Value {
    reply["choices"][0]["message"].clone()
}

#[test]
fn transforms_a_text_reply_into_the_openai_shape() {
    let output = success(
        &body(json!({"role": "assistant", "content": "hello"})),
        None,
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
        json_mode: None,
        ids: &FixedIds,
    };
    let reply = transform_response(input).expect("the response transform succeeds");
    assert_eq!(reply.model, "my-provider/m");
}

#[test]
fn an_empty_provider_falls_back_to_databricks() {
    let input = ResponseInput {
        body: r#"{"id": "i", "created": 1, "model": "m", "choices": [], "usage": {}}"#,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: Some(""),
        json_mode: None,
        ids: &FixedIds,
    };
    let reply = transform_response(input).expect("the response transform succeeds");
    assert_eq!(reply.model, "databricks/m");
}

#[test]
fn a_missing_model_becomes_an_empty_name() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .remove("model");
    assert_eq!(success(&value, None)["model"], json!("databricks/"));
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
#[case("id", json!("not-a-string"))]
#[case("created", json!(null))]
#[case("choices", json!({}))]
#[case("usage", json!([]))]
#[case("usage", json!("x"))]
fn a_broken_top_level_field_is_a_shape_error(#[case] field: &str, #[case] value: Value) {
    let mut body = body(json!({"role": "assistant", "content": "x"}));
    body.as_object_mut()
        .expect("the body is an object")
        .insert(field.to_owned(), value);
    assert_eq!(failure(&body, None).status_code(), 500);
}

#[rstest]
#[case(json!({"finish_reason": "stop"}), "message")]
#[case(json!({"message": {"role": "assistant", "content": "x"}}), "index")]
#[case(json!({"message": {"role": "assistant", "content": "x"}, "index": 0}), "finish_reason")]
#[case(json!({"message": {"role": "assistant"}, "index": 0, "finish_reason": "stop"}), "content")]
fn a_missing_choice_field_is_a_shape_error(#[case] choice: Value, #[case] name: &str) {
    let error = failure(&body_with(choice), None);
    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        format!("the Databricks response is missing its {name}")
    );
}

#[rstest]
#[case(json!(null), json!(0))]
#[case(json!(3), json!(3))]
fn the_choice_index_defaults_a_null(#[case] index: Value, #[case] expected: Value) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"index": index}),
    );
    assert_eq!(
        success(&body_with(choice), None)["choices"][0]["index"],
        expected
    );
}

#[test]
fn finish_reasons_are_mapped_and_keep_the_native_value() {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": "content_filtered"}),
    );
    let output = success(&body_with(choice), None);
    assert_eq!(
        output["choices"][0]["finish_reason"],
        json!("content_filter")
    );
    assert_eq!(
        output["choices"][0]["provider_specific_fields"],
        json!({"native_finish_reason": "content_filtered"})
    );
}

#[rstest]
#[case(json!(null), json!("stop"))]
#[case(json!("length"), json!("length"))]
#[case(json!("tool_calls"), json!("tool_calls"))]
fn a_known_or_null_finish_reason_adds_no_native_value(#[case] raw: Value, #[case] expected: Value) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": raw}),
    );
    let output = success(&body_with(choice), None);
    assert_eq!(output["choices"][0]["finish_reason"], expected);
    assert_eq!(output["choices"][0].get("provider_specific_fields"), None);
}

#[rstest]
#[case(json!(""))]
#[case(json!("weird"))]
fn an_unmapped_finish_reason_defaults_to_stop(#[case] raw: Value) {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": raw}),
    );
    let output = success(&body_with(choice), None);
    assert_eq!(output["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(
        output["choices"][0]["provider_specific_fields"],
        json!({"native_finish_reason": raw})
    );
}

#[test]
fn a_non_string_finish_reason_is_a_shape_error() {
    let choice = choice(
        json!({"role": "assistant", "content": "x"}),
        json!({"finish_reason": 5}),
    );
    let error = failure(&body_with(choice), None);
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
        message_of(&success(&body(message), None))["content"],
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
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["content"], json!("answer"));
    assert_eq!(output["reasoning_content"], json!("thinkmore"));
    assert_eq!(
        output["thinking_blocks"],
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
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["content"], json!(""));
    assert_eq!(
        output["thinking_blocks"],
        json!([{"type": "thinking", "thinking": "t", "signature": null}])
    );
}

#[test]
fn a_reasoning_summary_without_text_is_a_shape_error() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "reasoning", "summary": [{"signature": "s"}]}]
    });
    let error = failure(&body(message), None);
    assert_eq!(error.status_code(), 500);
}

#[test]
fn the_blocks_reasoning_wins_over_a_top_level_reasoning_content() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "reasoning", "summary": [{"text": "blk"}]}, {"type": "text", "text": "a"}],
        "reasoning_content": "top"
    });
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["reasoning_content"], json!("blk"));
    assert!(output.get("thinking_blocks").is_some());
}

#[test]
fn a_thinking_tag_is_split_without_a_reasoning_key() {
    let message = json!({"role": "assistant", "content": "<think>inside</think>rest"});
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["content"], json!("rest"));
    assert_eq!(output["reasoning_content"], json!("inside"));
    assert_eq!(output.get("thinking_blocks"), None);
}

#[test]
fn a_top_level_reasoning_content_skips_tag_parsing() {
    let message = json!({
        "role": "assistant",
        "content": "<think>inside</think>rest",
        "reasoning_content": "top"
    });
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["content"], json!("<think>inside</think>rest"));
    assert_eq!(output["reasoning_content"], json!("top"));
}

#[test]
fn a_null_reasoning_content_key_blocks_tag_parsing() {
    let message = json!({
        "role": "assistant",
        "content": "<think>inside</think>rest",
        "reasoning_content": null
    });
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["content"], json!("<think>inside</think>rest"));
    assert_eq!(output.get("reasoning_content"), None);
}

#[test]
fn a_reasoning_key_is_used_when_no_reasoning_content_is_set() {
    let message = json!({"role": "assistant", "content": "x", "reasoning": "why"});
    assert_eq!(
        message_of(&success(&body(message), None))["reasoning_content"],
        json!("why")
    );
}

#[rstest]
#[case(json!(5))]
#[case(json!(true))]
#[case(json!({"a": 1}))]
fn a_broken_content_value_is_a_shape_error(#[case] content: Value) {
    let message = json!({"role": "assistant", "content": content});
    assert_eq!(failure(&body(message), None).status_code(), 500);
}

#[test]
fn citations_land_in_provider_specific_fields() {
    let message = json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "answer", "citations": [{"start": 0, "end": 2}]}]
    });
    let output = message_of(&success(&body(message), None));
    assert_eq!(
        output["provider_specific_fields"],
        json!({"citations": [[{"start": 0, "end": 2, "supported_text": "answer"}]]})
    );
}

#[rstest]
#[case(json!([{"type": "text", "text": "a", "citations": []}]))]
#[case(json!([{"type": "text", "text": "a"}]))]
fn empty_citations_do_not_add_provider_specific_fields(#[case] content: Value) {
    let message = json!({"role": "assistant", "content": content});
    let output = message_of(&success(&body(message), None));
    assert_eq!(output.get("provider_specific_fields"), None);
}

#[test]
fn a_message_without_a_content_key_is_a_shape_error() {
    let error = failure(&body(json!({"role": "assistant"})), None);
    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        "the Databricks response is missing its content"
    );
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
    let output = message_of(&success(&body(message), None));
    assert_eq!(
        output["tool_calls"],
        json!([
            {"function": {"arguments": "{\"a\":1}", "name": "f"}, "id": "t1", "type": "function"},
            {"function": {"arguments": "", "name": "g"}, "id": "generated-id", "type": "function"},
            {"index": 3, "zz": 1, "function": {"arguments": "{\"b\":2}", "name": "h"}, "id": "t3", "type": "weird"}
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
    let output = message_of(&success(&body(message), None));
    assert_eq!(
        output["tool_calls"][0]["function"]["arguments"],
        json!("{\"a\":1}")
    );
}

#[rstest]
#[case(json!({"id": "t1"}), "the function of a tool call is missing")]
#[case(json!({"id": "t1", "function": null}), "the function of a tool call is missing")]
#[case(json!({"id": "t1", "function": "f"}), "the function of a tool call must be an object")]
#[case(json!({"id": 5, "function": {"name": "f"}}), "the id of a tool call must be a string")]
#[case(json!({"id": "t1", "type": 5, "function": {"name": "f"}}), "the type of a tool call must be a string")]
#[case(json!({"id": "t1", "function": {"name": 5}}), "the name of a tool call function must be a string")]
#[case(json!({"id": "t1", "function": {"name": "f", "arguments": 5}}), "the arguments of a tool call function must be a string or an object")]
#[case(json!({"id": "t1", "type": "custom", "custom": {}, "function": {"name": "f"}}), "a custom tool call in a Databricks reply is not supported")]
#[case(json!({"id": "t1", "custom": {"name": "f"}, "function": {"name": "f"}}), "a custom tool call in a Databricks reply is not supported")]
fn a_broken_tool_call_is_a_shape_error(#[case] tool_call: Value, #[case] message: &str) {
    let value = json!({"role": "assistant", "content": null, "tool_calls": [tool_call]});
    let error = failure(&body(value), None);
    assert_eq!(error.status_code(), 500);
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case(json!(null))]
#[case(json!([]))]
#[case(json!({}))]
#[case(json!(""))]
fn an_empty_tool_calls_value_becomes_null(#[case] tool_calls: Value) {
    let message = json!({"role": "assistant", "content": "x", "tool_calls": tool_calls});
    assert_eq!(
        message_of(&success(&body(message), None))["tool_calls"],
        json!(null)
    );
}

#[rstest]
#[case(json!("x"))]
#[case(json!(5))]
#[case(json!(true))]
#[case(json!({"a": 1}))]
fn a_non_list_tool_calls_value_is_a_shape_error(#[case] tool_calls: Value) {
    let message = json!({"role": "assistant", "content": "x", "tool_calls": tool_calls});
    assert_eq!(failure(&body(message), None).status_code(), 500);
}

fn json_mode_body(tool_calls: Value) -> Value {
    body(json!({"role": "assistant", "content": null, "tool_calls": tool_calls}))
}

#[test]
fn json_mode_turns_a_json_tool_call_into_content() {
    let tool_calls =
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(output["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(message_of(&output)["content"], json!("{\"a\":1}"));
    assert_eq!(message_of(&output)["tool_calls"], json!(null));
}

#[test]
fn json_mode_keeps_a_call_with_another_name() {
    let tool_calls = json!([{"id": "t1", "function": {"name": "f", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(message_of(&output)["content"], json!(null));
    assert!(message_of(&output)["tool_calls"].is_array());
}

#[test]
fn json_mode_keeps_two_calls() {
    let tool_calls = json!([
        {"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}},
        {"id": "t2", "function": {"name": "f", "arguments": "x"}}
    ]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(message_of(&output)["content"], json!(null));
    assert_eq!(
        message_of(&output)["tool_calls"].as_array().map(Vec::len),
        Some(2)
    );
}

#[test]
fn json_mode_without_the_flag_does_not_convert() {
    let tool_calls =
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{\"a\":1}"}}]);
    let output = success(&json_mode_body(tool_calls), Some(false));
    assert_eq!(message_of(&output)["content"], json!(null));
}

#[test]
fn json_mode_with_empty_arguments_does_not_convert() {
    let tool_calls = json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": ""}}]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(message_of(&output)["content"], json!(null));
    assert!(message_of(&output)["tool_calls"].is_array());
}

#[test]
fn json_mode_does_not_need_a_finish_reason() {
    let mut value = json_mode_body(
        json!([{"id": "t1", "function": {"name": "json_tool_call", "arguments": "{}"}}]),
    );
    value["choices"][0]
        .as_object_mut()
        .expect("the choice is an object")
        .remove("finish_reason");
    let output = success(&value, Some(true));
    assert_eq!(output["choices"][0]["finish_reason"], json!("stop"));
}

#[test]
fn json_mode_repairs_a_parallel_call_before_converting() {
    let arguments = json!({"tool_uses": [{"recipient_name": "functions.json_tool_call", "parameters": {"a": 1}}]});
    let tool_calls = json!([{
        "id": "t1",
        "function": {"name": "multi_tool_use.parallel", "arguments": arguments.to_string()}
    }]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(message_of(&output)["content"], json!("{\"a\":1}"));
    assert_eq!(message_of(&output)["tool_calls"], json!(null));
}

#[test]
fn a_parallel_call_with_invalid_json_is_left_alone() {
    let tool_calls = json!([{
        "id": "t1",
        "function": {"name": "multi_tool_use.parallel", "arguments": "not json"}
    }]);
    let output = success(&json_mode_body(tool_calls), Some(true));
    assert_eq!(message_of(&output)["content"], json!(null));
    assert_eq!(message_of(&output)["tool_calls"][0]["id"], json!("t1"));
}

#[rstest]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "5"}}), "the arguments of a multi_tool_use.parallel call must be an object")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{}"}}), "a multi_tool_use.parallel call is missing its tool_uses")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": {}}"}}), "the tool_uses of a multi_tool_use.parallel call must be a list")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": [\"x\"]}"}}), "the tool_use of a multi_tool_use.parallel call must be an object")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": [{\"parameters\": {}}]}"}}), "the recipient_name of a multi_tool_use.parallel tool_use is missing")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": [{\"recipient_name\": 5}]}"}}), "the recipient_name of a multi_tool_use.parallel tool_use must be a string")]
#[case(json!({"id": "t1", "function": {"name": "multi_tool_use.parallel", "arguments": "{\"tool_uses\": [{\"recipient_name\": \"f\"}]}"}}), "the parameters of a multi_tool_use.parallel tool_use is missing")]
fn a_broken_parallel_call_is_a_shape_error(#[case] tool_call: Value, #[case] message: &str) {
    let error = failure(&json_mode_body(json!([tool_call])), Some(true));
    assert_eq!(error.status_code(), 500);
    assert_eq!(error.to_string(), message);
}

#[test]
fn a_parallel_call_repair_runs_even_without_json_mode() {
    let tool_calls = json!([{
        "id": "t1",
        "function": {"name": "multi_tool_use.parallel", "arguments": "5"}
    }]);
    assert_eq!(
        failure(&json_mode_body(tool_calls), Some(false)).status_code(),
        500
    );
}

#[test]
fn usage_of_an_empty_block_is_zeroed() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert("usage".to_owned(), json!({}));
    assert_eq!(
        usage_of(&success(&value, None)),
        json!({
            "completion_tokens": 0,
            "prompt_tokens": 0,
            "total_tokens": 0,
            "completion_tokens_details": null,
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn reasoning_tokens_fill_the_completion_details() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value.as_object_mut().expect("the body is an object").insert(
        "usage".to_owned(),
        json!({"prompt_tokens": 1, "completion_tokens": 10, "total_tokens": 11, "reasoning_tokens": 3,
               "completion_tokens_details": {"image_tokens": 2, "audio_tokens": 1}}),
    );
    assert_eq!(
        usage_of(&success(&value, None)),
        json!({
            "completion_tokens": 10,
            "prompt_tokens": 1,
            "total_tokens": 11,
            "completion_tokens_details": {"image_tokens": 2, "audio_tokens": 1, "reasoning_tokens": 3, "text_tokens": 4},
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn reasoning_tokens_keep_a_provider_text_tokens_value() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value.as_object_mut().expect("the body is an object").insert(
        "usage".to_owned(),
        json!({"prompt_tokens": 1, "completion_tokens": 10, "total_tokens": 11, "reasoning_tokens": 3,
               "completion_tokens_details": {"text_tokens": 99}}),
    );
    assert_eq!(
        usage_of(&success(&value, None))["completion_tokens_details"],
        json!({"text_tokens": 99, "reasoning_tokens": 3})
    );
}

#[test]
fn reasoning_tokens_without_completion_tokens_skip_text_tokens() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value.as_object_mut().expect("the body is an object").insert(
        "usage".to_owned(),
        json!({"prompt_tokens": 1, "completion_tokens": null, "total_tokens": 3, "reasoning_tokens": 3}),
    );
    assert_eq!(
        usage_of(&success(&value, None)),
        json!({
            "completion_tokens": 0,
            "prompt_tokens": 1,
            "total_tokens": 3,
            "completion_tokens_details": {"reasoning_tokens": 3},
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn text_tokens_clamp_at_zero() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value.as_object_mut().expect("the body is an object").insert(
        "usage".to_owned(),
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3, "reasoning_tokens": 5}),
    );
    assert_eq!(
        usage_of(&success(&value, None))["completion_tokens_details"],
        json!({"reasoning_tokens": 5, "text_tokens": 0})
    );
}

#[test]
fn cache_keys_fold_into_the_prompt_details_and_stay_on_the_usage() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value.as_object_mut().expect("the body is an object").insert(
        "usage".to_owned(),
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "cache_read_input_tokens": 9, "cache_creation_input_tokens": 4, "prompt_cache_hit_tokens": 2}),
    );
    assert_eq!(
        usage_of(&success(&value, None)),
        json!({
            "completion_tokens": 2,
            "prompt_tokens": 1,
            "total_tokens": 3,
            "completion_tokens_details": null,
            "prompt_tokens_details": {"cached_tokens": 9, "cache_write_tokens": 4, "cache_creation_tokens": 4},
            "cache_read_input_tokens": 9,
            "cache_creation_input_tokens": 4,
            "prompt_cache_hit_tokens": 2
        })
    );
}

#[test]
fn a_nested_cache_creation_key_fills_the_write_tokens() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert(
            "usage".to_owned(),
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "prompt_tokens_details": {"cache_creation_input_tokens": 3}}),
        );
    assert_eq!(
        usage_of(&success(&value, None))["prompt_tokens_details"],
        json!({"cache_creation_input_tokens": 3, "cache_write_tokens": 3, "cache_creation_tokens": 3})
    );
}

#[test]
fn prompt_details_drop_their_unset_optional_fields() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert(
            "usage".to_owned(),
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "prompt_tokens_details": {"character_count": null, "cached_tokens": 5, "zz": 1}}),
        );
    assert_eq!(
        usage_of(&success(&value, None))["prompt_tokens_details"],
        json!({"cached_tokens": 5, "zz": 1})
    );
}

#[rstest]
#[case(json!({}))]
#[case(json!("x"))]
#[case(json!([1]))]
fn a_truthy_non_object_details_value_is_dropped(#[case] details: Value) {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert(
            "usage".to_owned(),
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "completion_tokens_details": details, "prompt_tokens_details": details}),
        );
    let output = usage_of(&success(&value, None));
    assert_eq!(output["completion_tokens_details"], json!(null));
    assert_eq!(output["prompt_tokens_details"], json!(null));
}

#[test]
fn server_tool_use_keeps_its_declared_fields_and_cost_is_forwarded() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert(
            "usage".to_owned(),
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "server_tool_use": {"web_search_requests": 2, "zz": 1}, "cost": 0.5, "zz": null}),
        );
    let output = usage_of(&success(&value, None));
    assert_eq!(output["server_tool_use"], json!({"web_search_requests": 2}));
    assert_eq!(output["cost"], json!(0.5));
    assert_eq!(output["zz"], json!(null));
}

#[test]
fn a_null_server_tool_use_and_cost_are_dropped() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    value
        .as_object_mut()
        .expect("the body is an object")
        .insert(
            "usage".to_owned(),
            json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "server_tool_use": null, "cost": null}),
        );
    let output = usage_of(&success(&value, None));
    assert_eq!(output.get("server_tool_use"), None);
    assert_eq!(output.get("cost"), None);
}

#[rstest]
#[case("prompt_tokens", json!("5"))]
#[case("completion_tokens", json!(2.5))]
#[case("total_tokens", json!(true))]
#[case("reasoning_tokens", json!("3"))]
fn a_wrong_typed_usage_count_is_a_shape_error(#[case] name: &str, #[case] value: Value) {
    let mut body = body(json!({"role": "assistant", "content": "x"}));
    let mut usage = json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3});
    usage
        .as_object_mut()
        .expect("the usage is an object")
        .insert(name.to_owned(), value);
    body.as_object_mut()
        .expect("the body is an object")
        .insert("usage".to_owned(), usage);
    assert_eq!(failure(&body, None).status_code(), 500);
}

#[test]
fn a_message_level_function_call_is_dropped() {
    let message = json!({
        "role": "assistant",
        "content": "x",
        "function_call": {"name": "f", "arguments": "{}"}
    });
    let output = message_of(&success(&body(message), None));
    assert_eq!(output["function_call"], json!(null));
}
