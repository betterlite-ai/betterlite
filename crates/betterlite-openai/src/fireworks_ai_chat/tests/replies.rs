use betterlite_models::decisions::Json as HiddenJson;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde_json::{Number, json};

use super::super::error::FireworksChatError;
use super::super::reply::{Reply, ReplyInput, transform_response};
use super::super::{JSON_TOOL_NAME, MODEL_PREFIX};
use super::{fixed_clock, fixed_id, response_headers};
use crate::chat::ToolCall;
use crate::chat::response::ChatUsage;

fn reply_body(body: &[u8], tools: Option<&Json>) -> Result<Reply, FireworksChatError> {
    let headers = response_headers(&[("x-request-id", "req-1")]);
    transform_response(&ReplyInput {
        status: 200,
        headers: &headers,
        body,
        tools,
        identifier: fixed_id,
        clock: fixed_clock,
    })
}

fn declared() -> Json {
    serde_json::from_value(json!([{"type": "function", "function": {"name": "lookup"}}]))
        .expect("declared tools")
}

fn choice_body(content: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1_700_000_000_i64,
        "model": "llama-v3",
        "choices": [{
            "index": 0_i64,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": content}
        }]
    }))
    .expect("body JSON")
}

fn repaired_arguments(content: &str) -> Option<String> {
    let tools = declared();
    let reply = reply_body(&choice_body(content), Some(&tools)).expect("reply");
    let choice = &reply.response.choices[0];
    let calls: &Vec<ToolCall> = choice.message.tool_calls.as_ref()?;
    assert_eq!(choice.message.content, None);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, fixed_id().to_string());
    assert_eq!(calls[0].kind, "function");
    assert_eq!(calls[0].function.name, "lookup");
    Some(calls[0].function.arguments.clone())
}

fn content_after_repair(content: &str) -> Option<String> {
    let tools = declared();
    let reply = reply_body(&choice_body(content), Some(&tools)).expect("reply");
    let choice = &reply.response.choices[0];
    assert!(choice.message.tool_calls.is_none());
    choice.message.content.clone()
}

fn tool_calls_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1_700_000_000_i64,
        "model": "llama-v3",
        "choices": [{
            "index": 0_i64,
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": r#"{"name":"lookup","arguments":"{}"}"#,
                "tool_calls": []
            }
        }]
    }))
    .expect("body JSON")
}

fn tool_calls_body_with_one_call() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1_700_000_000_i64,
        "model": "llama-v3",
        "choices": [{
            "index": 0_i64,
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": r#"{"name":"lookup","arguments":"{}"}"#,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {"name": "search", "arguments": "{}"}
                }]
            }
        }]
    }))
    .expect("body JSON")
}

#[test]
fn non_json_reply_is_a_fireworks_error_with_status_and_headers() {
    let headers = response_headers(&[("x-request-id", "req-1")]);
    let error = transform_response(&ReplyInput {
        status: 502,
        headers: &headers,
        body: b"not json",
        tools: None,
        identifier: fixed_id,
        clock: fixed_clock,
    })
    .expect_err("not JSON");
    assert_eq!(error.status_code(), Some(502));
    let message = error.to_string();
    assert!(message.starts_with("Unable to get json response - "));
    assert!(message.ends_with(", Original Response: not json"));
    let FireworksChatError::Response(response_error) = error else {
        panic!("a Fireworks response error");
    };
    assert_eq!(response_error.status_code, 502);
    assert_eq!(
        response_error
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-request-id")),
        Some(&String::from("req-1"))
    );
}

#[test]
fn reply_model_gets_the_fireworks_prefix() {
    let reply = reply_body(
        br#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"llama-v3","choices":[]}"#,
        None,
    )
    .expect("reply");
    assert_eq!(reply.response.model, format!("{MODEL_PREFIX}llama-v3"));
    assert_eq!(reply.response.object, "chat.completion");
}

#[test]
fn reply_forces_the_chat_completion_object() {
    let reply = reply_body(
        br#"{"id":"chatcmpl-1","object":"chat.completion.custom","created":1700000000,"model":"llama-v3","choices":[]}"#,
        None,
    )
    .expect("reply");
    assert_eq!(reply.response.object, "chat.completion");
}

#[test]
fn reply_prefixes_an_explicit_empty_model() {
    let reply = reply_body(
        br#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"","choices":[]}"#,
        None,
    )
    .expect("reply");
    assert_eq!(reply.response.model, MODEL_PREFIX);
}

#[test]
fn reply_without_a_model_leaves_it_empty() {
    let reply = reply_body(br#"{"choices":[]}"#, None).expect("reply");
    assert_eq!(reply.response.model, "");
}

#[test]
fn reply_defaults_an_absent_id_created_and_object() {
    let reply = reply_body(br#"{"model":"llama-v3","choices":[]}"#, None).expect("reply");
    assert_eq!(reply.response.id, format!("chatcmpl-{}", fixed_id()));
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.object, "chat.completion");
}

#[test]
fn reply_keeps_a_given_id_and_created() {
    let reply = reply_body(
        br#"{"id":"chatcmpl-given","created":1,"model":"llama-v3","choices":[]}"#,
        None,
    )
    .expect("reply");
    assert_eq!(reply.response.id, "chatcmpl-given");
    assert_eq!(reply.response.created, 1);
}

#[test]
fn reply_reads_a_null_usage_as_none() {
    let reply = reply_body(
        br#"{"id":"chatcmpl-1","created":1700000000,"model":"llama-v3","usage":null,"choices":[]}"#,
        None,
    )
    .expect("reply");
    assert_eq!(reply.response.usage, ChatUsage::default());
}

#[test]
fn content_tool_call_becomes_a_tool_call_for_a_declared_name() {
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}"#),
        Some(String::from(r#"{"city":"Paris"}"#))
    );
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup","parameters":{"city":"Paris","limit":2}}"#),
        Some(String::from(r#"{"city":"Paris","limit":2}"#))
    );
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup","parameters":{}}"#),
        Some(String::from("{}"))
    );
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup"}"#),
        Some(String::new())
    );
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup","arguments":null,"parameters":{"city":"Paris"}}"#),
        Some(String::from(r#"{"city":"Paris"}"#))
    );
    assert_eq!(
        repaired_arguments(r#"{"name":"lookup","parameters":"text"}"#),
        Some(String::new())
    );
}

#[test]
fn content_that_is_not_a_declared_function_stays_content() {
    for content in [
        r#"{"name":"json_tool_call","arguments":"{}"}"#,
        r#"{"name":"undeclared","arguments":"{}"}"#,
        "not json",
        "[]",
        "null",
        r#"{"name":"lookup","arguments":4}"#,
        r#"{"name":4,"arguments":"{}"}"#,
        r#"{"name":"lookup","arguments":[]}"#,
    ] {
        assert_eq!(
            content_after_repair(content),
            Some(content.to_owned()),
            "{content}"
        );
    }
}

#[test]
fn a_declared_json_tool_call_function_is_never_converted() {
    let tools: Json = serde_json::from_value(json!([
        {"type": "function", "function": {"name": JSON_TOOL_NAME}}
    ]))
    .expect("declared tools");
    let content = format!(r#"{{"name":"{JSON_TOOL_NAME}","arguments":"{{}}"}}"#);
    let reply = reply_body(&choice_body(&content), Some(&tools)).expect("reply");
    let choice = &reply.response.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some(content.as_str()));
    assert!(choice.message.tool_calls.is_none());
}

#[test]
fn content_reparsing_treats_an_empty_tool_calls_list_as_absent() {
    let tools = declared();
    let reply = reply_body(&tool_calls_body(), Some(&tools)).expect("reply");
    let choice = &reply.response.choices[0];
    assert_eq!(choice.message.content, None);
    assert_eq!(choice.message.tool_calls.as_ref().map(Vec::len), Some(1));
}

#[test]
fn content_reparsing_skips_a_non_empty_tool_calls_list() {
    let tools = declared();
    let reply = reply_body(&tool_calls_body_with_one_call(), Some(&tools)).expect("reply");
    let choice = &reply.response.choices[0];
    assert_eq!(
        choice.message.content.as_deref(),
        Some(r#"{"name":"lookup","arguments":"{}"}"#)
    );
    assert_eq!(choice.message.tool_calls.as_ref().map(Vec::len), Some(1));
}

#[test]
fn content_reparsing_needs_tools() {
    let reply =
        reply_body(&choice_body(r#"{"name":"lookup","arguments":"{}"}"#), None).expect("reply");
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some(r#"{"name":"lookup","arguments":"{}"}"#)
    );
}

#[test]
fn content_reparsing_is_skipped_for_a_malformed_tools_value() {
    let tools = serde_json::from_value(json!([{"type": "function"}])).expect("tools");
    let reply = reply_body(
        &choice_body(r#"{"name":"lookup","arguments":"{}"}"#),
        Some(&tools),
    )
    .expect("reply");
    assert!(reply.response.choices[0].message.tool_calls.is_none());
}

#[test]
fn content_reparsing_is_skipped_for_a_null_tools_value() {
    let tools = Json::Null;
    let reply = reply_body(
        &choice_body(r#"{"name":"lookup","arguments":"{}"}"#),
        Some(&tools),
    )
    .expect("reply");
    assert!(reply.response.choices[0].message.tool_calls.is_none());
}

#[test]
fn reply_finish_reasons_are_mapped_with_the_native_value() {
    let body = br#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"llama-v3","choices":[{"index":0,"finish_reason":"max_tokens","message":{"role":"assistant","content":"cut"}}]}"#;
    let reply = reply_body(body, None).expect("reply");
    let choice = &reply.response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        choice.provider_specific_fields.get("native_finish_reason"),
        Some(&json!("max_tokens"))
    );
}

#[test]
fn reply_finish_reasons_default_to_stop() {
    let body = br#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"llama-v3","choices":[{"index":0,"message":{"role":"assistant","content":"done"}}]}"#;
    let reply = reply_body(body, None).expect("reply");
    assert_eq!(
        reply.response.choices[0].finish_reason.as_deref(),
        Some("stop")
    );
    assert!(
        reply.response.choices[0]
            .provider_specific_fields
            .is_empty()
    );
}

#[test]
fn reply_keeps_a_mapped_finish_reason_without_a_native_value() {
    let body = br#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"llama-v3","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":"done"}}]}"#;
    let reply = reply_body(body, None).expect("reply");
    assert_eq!(
        reply.response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    assert!(
        reply.response.choices[0]
            .provider_specific_fields
            .is_empty()
    );
}

#[test]
fn reply_hidden_params_collect_the_top_level_fireworks_fields() {
    let body = br#"{
        "id":"chatcmpl-1","created":1700000000,"model":"llama-v3",
        "perf_metrics":{"request_id":"r1"},
        "prompt_token_ids":null,
        "choices":[]
    }"#;
    let reply = reply_body(body, None).expect("reply");
    assert_eq!(
        reply.hidden_params.get("fireworks_perf_metrics"),
        Some(&HiddenJson::Object(IndexMap::from([(
            String::from("request_id"),
            HiddenJson::Text(String::from("r1"))
        )])))
    );
    assert_eq!(
        reply.hidden_params.get("fireworks_prompt_token_ids"),
        Some(&HiddenJson::Null)
    );
}

#[test]
fn reply_hidden_params_collect_the_per_choice_fields() {
    let body = br#"{
        "id":"chatcmpl-1","created":1700000000,"model":"llama-v3",
        "choices":[
            {"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"a"},"raw_output":{"seen":1}},
            {"index":1,"finish_reason":"stop","message":{"role":"assistant","content":"b"},"token_ids":[4,5]}
        ]
    }"#;
    let reply = reply_body(body, None).expect("reply");
    assert_eq!(
        reply.hidden_params.get("fireworks_raw_outputs"),
        Some(&HiddenJson::List(vec![HiddenJson::Object(IndexMap::from(
            [(
                String::from("seen"),
                HiddenJson::Number(Number::from(1_i64))
            )]
        ))]))
    );
    assert_eq!(
        reply.hidden_params.get("fireworks_token_ids"),
        Some(&HiddenJson::List(vec![HiddenJson::List(vec![
            HiddenJson::Number(Number::from(4_i64)),
            HiddenJson::Number(Number::from(5_i64))
        ])]))
    );
}

#[test]
fn reply_hidden_params_omit_choice_fields_no_choice_carries() {
    let body = br#"{"id":"chatcmpl-1","created":1700000000,"model":"llama-v3","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"a"}}]}"#;
    let reply = reply_body(body, None).expect("reply");
    assert!(reply.hidden_params.get("fireworks_raw_outputs").is_none());
    assert!(reply.hidden_params.get("fireworks_token_ids").is_none());
    assert!(reply.hidden_params.get("fireworks_perf_metrics").is_none());
    assert!(
        reply
            .hidden_params
            .get("fireworks_prompt_token_ids")
            .is_none()
    );
}

#[test]
fn reply_hidden_params_carry_the_projected_response_headers() {
    let body = br#"{"id":"chatcmpl-1","created":1700000000,"model":"llama-v3","choices":[]}"#;
    let reply = reply_body(body, None).expect("reply");
    let Some(HiddenJson::Object(projected)) = reply.hidden_params.get("additional_headers") else {
        panic!("additional headers");
    };
    assert_eq!(
        projected.get("llm_provider-x-request-id"),
        Some(&HiddenJson::Text(String::from("req-1")))
    );
    assert!(!projected.contains_key("x-request-id"));
}
