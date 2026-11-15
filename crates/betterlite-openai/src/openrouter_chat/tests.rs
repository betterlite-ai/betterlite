#![cfg(test)]

use std::sync::{LazyLock, Mutex};

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};

use super::cache_control::{move_cache_control_to_content, supports_cache_control};
use super::error::get_error_class;
use super::policy::{BaseRequest, ChatPolicy};
use super::stream::chunk_parser;
use super::{ChatReply, ChatRequest, OpenrouterConfig};
use crate::chat::{ChatMessage, MessageContent};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("the fixture is JSON")
}

#[derive(Default)]
struct FakePolicy {
    calls: Mutex<Vec<&'static str>>,
    unsupported_cost: bool,
    base_already_has_thinking: bool,
}

impl FakePolicy {
    fn record(&self, name: &'static str) {
        self.calls.lock().unwrap().push(name);
    }
}

fn as_prompt_json(value: Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(flag),
        Value::Number(number) => Json::Number(number),
        Value::String(text) => Json::Text(text),
        Value::Array(items) => Json::List(items.into_iter().map(as_prompt_json).collect()),
        Value::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, item)| (name, as_prompt_json(item)))
                .collect(),
        ),
    }
}

impl ChatPolicy for FakePolicy {
    fn supported_params(&self, _model: &str) -> Vec<&'static str> {
        if self.base_already_has_thinking {
            return vec!["thinking", "stream"];
        }
        vec!["stream", "temperature"]
    }

    fn map_params(&self, model: &str, requested: &Object, optional_params: &mut Object) {
        self.record("map_params");
        let supported = self.supported_params(model);
        for (name, value) in requested {
            if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
    }

    fn remove_cache_control(&self, messages: &mut Vec<ChatMessage>, optional_params: &mut Object) {
        self.record("remove_cache_control");
        for message in messages {
            message.passthrough.shift_remove("cache_control");
        }
        optional_params.shift_remove("tools");
    }

    fn request_body(&self, request: BaseRequest<'_>) -> Result<Object, OpenRouterError> {
        self.record("request_body");
        let mut body = Object::new();
        body.insert(String::from("model"), Json::Text(request.model.to_owned()));
        let messages =
            serde_json::to_value(request.messages).map_err(|source| OpenRouterError {
                message: source.to_string(),
                status_code: 400,
                headers: Headers::new(),
            })?;
        body.insert(String::from("messages"), as_prompt_json(messages));
        for (name, value) in request.optional_params {
            body.insert(name.clone(), value.clone());
        }
        Ok(body)
    }

    fn response_body(&self, raw: &[u8]) -> Result<Value, OpenRouterError> {
        self.record("response_body");
        if self.unsupported_cost {
            return Err(OpenRouterError {
                message: String::from("the base response build failed"),
                status_code: 400,
                headers: Headers::new(),
            });
        }
        serde_json::from_slice(raw).map_err(|_source| OpenRouterError {
            message: String::from("the provider answer is not valid JSON"),
            status_code: 400,
            headers: Headers::new(),
        })
    }
}

fn config() -> OpenrouterConfig<FakePolicy> {
    OpenrouterConfig {
        policy: FakePolicy::default(),
    }
}

fn message(value: Value) -> ChatMessage {
    serde_json::from_value(value).unwrap()
}

fn object(value: Value) -> Object {
    match as_prompt_json(value) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("test fixture is not an object")
        }
    }
}

static EMPTY_PARAMS: LazyLock<Object> = LazyLock::new(Object::new);
static EMPTY_HEADERS: LazyLock<Headers> = LazyLock::new(Headers::new);

fn request<'request>(
    model: &'request str,
    messages: &'request [ChatMessage],
    optional_params: &'request Object,
) -> ChatRequest<'request> {
    ChatRequest {
        model,
        messages,
        optional_params,
        litellm_params: &EMPTY_PARAMS,
        headers: &EMPTY_HEADERS,
    }
}

#[test]
fn supported_params_appends_each_openrouter_param_once() {
    let config = config();
    let params = config.supported_params("openai/gpt-4o");
    assert_eq!(
        params,
        vec!["stream", "temperature", "reasoning_effort", "thinking"]
    );
}

#[test]
fn supported_params_keeps_a_base_entry_only_once() {
    let policy = FakePolicy {
        base_already_has_thinking: true,
        ..FakePolicy::default()
    };
    let config = OpenrouterConfig { policy };
    assert_eq!(
        config.supported_params("m"),
        vec!["thinking", "stream", "reasoning_effort"]
    );
}

#[test]
fn map_openai_params_renames_max_and_moves_the_openrouter_keys() {
    let config = config();
    let requested = object(parsed(
        r#"{"reasoning_effort": "max", "transforms": ["middle-out"], "models": ["a/b"], "route": "fallback", "provider": {"order": ["x"]}, "temperature": 0.2}"#,
    ));
    let mut optional = Object::new();
    config.map_openai_params("openrouter/x", &requested, &mut optional);
    assert_eq!(
        optional.get("extra_body"),
        Some(&Json::Object(object(parsed(
            r#"{"transforms": ["middle-out"], "models": ["a/b"], "route": "fallback"}"#
        ))))
    );
    assert_eq!(
        optional.get("reasoning_effort"),
        Some(&Json::Text(String::from("xhigh")))
    );
    assert_eq!(
        optional.get("temperature"),
        Some(&Json::Number(serde_json::Number::from_f64(0.2).unwrap()))
    );
    assert!(!optional.contains_key("provider"));
}

#[test]
fn map_openai_params_always_adds_an_extra_body() {
    let config = config();
    let mut optional = Object::new();
    config.map_openai_params("openrouter/x", &Object::new(), &mut optional);
    assert_eq!(
        optional.get("extra_body"),
        Some(&Json::Object(Object::new()))
    );
}

#[test]
fn map_openai_params_drops_a_null_openrouter_key() {
    let config = config();
    let mut optional = Object::new();
    config.map_openai_params(
        "openrouter/x",
        &object(parsed(
            r#"{"transforms": null, "models": ["m"], "route": null}"#,
        )),
        &mut optional,
    );
    assert_eq!(
        optional.get("extra_body"),
        Some(&Json::Object(object(parsed(r#"{"models": ["m"]}"#))))
    );
}

#[test]
fn map_openai_params_copies_the_openrouter_only_params() {
    let config = config();
    let requested = object(parsed(
        r#"{"reasoning_effort": "low", "thinking": {"type": "enabled"}, "stream": true, "provider": {"order": ["x"]}}"#,
    ));
    let mut optional = Object::new();
    config.map_openai_params("openrouter/x", &requested, &mut optional);
    assert_eq!(
        optional.get("reasoning_effort"),
        Some(&Json::Text(String::from("low")))
    );
    assert_eq!(
        optional.get("thinking"),
        Some(&Json::Object(object(parsed(r#"{"type": "enabled"}"#))))
    );
    assert_eq!(optional.get("stream"), Some(&Json::Bool(true)));
    assert!(!optional.contains_key("provider"));
}

#[rstest::rstest]
#[case::claude("anthropic/claude-3.5-sonnet")]
#[case::uppercase("Anthropic/CLAUDE-3-Haiku")]
#[case::gemini("google/gemini-2.5-pro")]
#[case::minimax("minimax/m2")]
#[case::glm("z-ai/glm-4.5")]
#[case::substring("my-glm-model")]
#[case::zai("z-ai/auto")]
fn cache_control_models_match_a_lowercased_substring(#[case] model: &str) {
    assert!(supports_cache_control(model));
}

#[rstest::rstest]
#[case::plain_openai("openai/gpt-4o")]
#[case::z_dot_ai("z.ai/auto")]
#[case::empty("")]
fn other_models_do_not_support_cache_control(#[case] model: &str) {
    assert!(!supports_cache_control(model));
}

#[test]
fn move_cache_control_puts_the_flag_on_the_last_block() {
    let messages = vec![message(parsed(
        r#"{"role": "user", "cache_control": {"type": "ephemeral"}, "content": [{"type": "text", "text": "first"}, {"type": "text", "text": "second"}]}"#,
    ))];
    let moved = move_cache_control_to_content(&messages);
    let Json::List(blocks) = block_list(moved[0].content.as_ref().unwrap()) else {
        panic!("content is not a block list");
    };
    assert_eq!(blocks.len(), 2);
    let Json::Object(first) = &blocks[0] else {
        panic!("block is not an object");
    };
    assert!(first.get("cache_control").is_none());
    let Json::Object(last) = &blocks[1] else {
        panic!("block is not an object");
    };
    assert_eq!(
        last.get("cache_control"),
        Some(&Json::Object(object(parsed(r#"{"type": "ephemeral"}"#))))
    );
    assert!(!moved[0].passthrough.contains_key("cache_control"));
}

fn block_list(content: &MessageContent) -> Json {
    as_prompt_json(serde_json::to_value(content).unwrap())
}

#[test]
fn move_cache_control_wraps_text_content_in_one_block() {
    let messages = vec![message(parsed(
        r#"{"role": "user", "content": "hello", "cache_control": {"type": "ephemeral"}}"#,
    ))];
    let moved = move_cache_control_to_content(&messages);
    assert_eq!(
        block_list(moved[0].content.as_ref().unwrap()),
        Json::List(vec![Json::Object(object(parsed(
            r#"{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}}"#
        )))])
    );
}

#[test]
fn move_cache_control_wraps_null_assistant_content() {
    let messages = vec![message(parsed(
        r#"{"role": "assistant", "content": null, "tool_calls": [{"id": "t", "type": "function", "function": {"name": "f", "arguments": "{}"}}], "cache_control": {"type": "ephemeral"}}"#,
    ))];
    let moved = move_cache_control_to_content(&messages);
    assert_eq!(
        block_list(moved[0].content.as_ref().unwrap()),
        Json::List(vec![Json::Object(object(parsed(
            r#"{"type": "text", "text": null, "cache_control": {"type": "ephemeral"}}"#
        )))])
    );
}

#[test]
fn move_cache_control_keeps_content_behind_a_null_flag() {
    let messages = vec![message(parsed(
        r#"{"role": "user", "content": "hello", "cache_control": null}"#,
    ))];
    let moved = move_cache_control_to_content(&messages);
    assert!(!moved[0].passthrough.contains_key("cache_control"));
    assert_eq!(
        moved[0].content,
        Some(MessageContent::Text(String::from("hello")))
    );
}

#[test]
fn move_cache_control_leaves_an_empty_block_list_empty() {
    let messages = vec![message(parsed(
        r#"{"role": "user", "content": [], "cache_control": {"type": "ephemeral"}}"#,
    ))];
    let moved = move_cache_control_to_content(&messages);
    assert_eq!(
        block_list(moved[0].content.as_ref().unwrap()),
        Json::List(Vec::new())
    );
    assert!(!moved[0].passthrough.contains_key("cache_control"));
}

#[test]
fn transform_request_asks_the_base_to_strip_only_unsupported_models() {
    let config = config();
    let messages = vec![message(parsed(
        r#"{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}"#,
    ))];
    let optional = Object::new();
    config
        .transform_request(&request(
            "anthropic/claude-3.5-sonnet",
            &messages,
            &optional,
        ))
        .unwrap();
    assert!(
        !config
            .policy
            .calls
            .lock()
            .unwrap()
            .contains(&"remove_cache_control")
    );
    config
        .transform_request(&request("openai/gpt-4o", &messages, &optional))
        .unwrap();
    assert!(
        config
            .policy
            .calls
            .lock()
            .unwrap()
            .contains(&"remove_cache_control")
    );
}

#[test]
fn transform_request_merges_extra_body_over_the_base_body() {
    let config = config();
    let optional = object(parsed(
        r#"{"extra_body": {"model": "overridden/model", "usage": {"include": false}}, "temperature": 0.1}"#,
    ));
    let body = config
        .transform_request(&request("openai/gpt-4o", &[], &optional))
        .unwrap();
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("overridden/model")))
    );
    assert_eq!(
        body.get("usage"),
        Some(&Json::Object(object(parsed(r#"{"include": false}"#))))
    );
    assert!(!body.contains_key("extra_body"));
}

#[test]
fn transform_request_adds_the_usage_include_default() {
    let config = config();
    let body = config
        .transform_request(&request("openai/gpt-4o", &[], &Object::new()))
        .unwrap();
    assert_eq!(
        body.get("usage"),
        Some(&Json::Object(object(parsed(r#"{"include": true}"#))))
    );
}

#[test]
fn transform_request_rejects_a_non_object_extra_body() {
    let config = config();
    let optional = object(parsed(r#"{"extra_body": "nope"}"#));
    let error = config
        .transform_request(&request("openai/gpt-4o", &[], &optional))
        .unwrap_err();
    assert_eq!(error.status_code, 400);
    assert!(error.message.contains("extra_body"));
}

#[test]
fn transform_response_adds_the_cost_header_and_keeps_other_headers() {
    let config = config();
    let mut headers = AdditionalHeaders::new();
    headers.insert(
        String::from("llm_provider-x-request-id"),
        HiddenJson::Text(String::from("req")),
    );
    let reply: ChatReply = config
        .transform_response(br#"{"usage":{"cost":0.5}}"#, headers)
        .unwrap();
    let cost = reply
        .additional_headers
        .get("llm_provider-x-litellm-response-cost")
        .unwrap();
    assert_eq!(
        *cost,
        HiddenJson::Number(serde_json::Number::from_f64(0.5).unwrap())
    );
    assert!(
        reply
            .additional_headers
            .contains_key("llm_provider-x-request-id")
    );
}

#[rstest::rstest]
#[case::missing_usage(r#"{"id":"x"}"#)]
#[case::null_usage(r#"{"usage":null}"#)]
#[case::empty_usage(r#"{"usage":{}}"#)]
#[case::null_cost(r#"{"usage":{"cost":null}}"#)]
#[case::string_cost(r#"{"usage":{"cost":"0.5"}}"#)]
#[case::boolean_cost(r#"{"usage":{"cost":true}}"#)]
fn transform_response_leaves_no_cost_header(#[case] raw: &str) {
    let config = config();
    let reply = config
        .transform_response(raw.as_bytes(), AdditionalHeaders::new())
        .unwrap();
    assert!(
        !reply
            .additional_headers
            .contains_key("llm_provider-x-litellm-response-cost")
    );
}

#[test]
fn reported_cost_ignores_a_body_it_cannot_read() {
    assert_eq!(super::reported_cost(b"not json"), None);
}

#[test]
fn transform_response_propagates_a_base_failure() {
    let policy = FakePolicy {
        unsupported_cost: true,
        ..FakePolicy::default()
    };
    let config = OpenrouterConfig { policy };
    let error = config
        .transform_response(b"{}", AdditionalHeaders::new())
        .unwrap_err();
    assert_eq!(error.message, "the base response build failed");
}

#[test]
fn stream_chunk_copies_reasoning_into_reasoning_content() {
    let chunk = parsed(
        r#"{"id": "chunk-1", "created": 7, "model": "anthropic/claude-3.5-sonnet", "choices": [{"index": 0, "delta": {"reasoning": "think", "content": "hi"}, "finish_reason": null}, {"index": 1, "delta": {"content": "plain"}}], "usage": {"prompt_tokens": 3}}"#,
    );
    let reply = chunk_parser(&chunk).unwrap();
    assert_eq!(reply.id, "chunk-1");
    assert_eq!(reply.object, "chat.completion.chunk");
    assert_eq!(reply.created, 7);
    assert_eq!(reply.model.as_deref(), Some("anthropic/claude-3.5-sonnet"));
    assert_eq!(
        reply.choices[0].delta.reasoning_content.as_deref(),
        Some("think")
    );
    assert_eq!(
        reply.choices[0]
            .delta
            .passthrough
            .get("reasoning")
            .and_then(Value::as_str),
        Some("think")
    );
    assert_eq!(reply.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(reply.choices[1].delta.reasoning_content, None);
    assert!(!reply.choices[1].delta.passthrough.contains_key("reasoning"));
    assert_eq!(
        reply.usage,
        Some(parsed(
            r#"{"prompt_tokens": 3, "completion_tokens": 0, "total_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null}"#,
        ))
    );
}

#[test]
fn stream_chunk_keeps_an_empty_reasoning_beside_its_copy() {
    let chunk = parsed(
        r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": {"reasoning": ""}}]}"#,
    );
    let reply = chunk_parser(&chunk).unwrap();
    assert_eq!(
        reply.choices[0].delta.reasoning_content.as_deref(),
        Some("")
    );
    assert_eq!(
        reply.choices[0]
            .delta
            .passthrough
            .get("reasoning")
            .and_then(Value::as_str),
        Some("")
    );
}

#[test]
fn stream_chunk_maps_finish_reason_and_numbers_tool_calls() {
    let chunk = parsed(
        r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": {"tool_calls": [{"id": "call_1", "function": {"name": "f", "arguments": ""}}, {"id": "call_2", "index": 5, "function": {"name": "g", "arguments": "{}"}}]}, "finish_reason": "tool_use"}, {"index": 1, "delta": {"content": "x"}, "finish_reason": ""}]}"#,
    );
    let reply = chunk_parser(&chunk).unwrap();
    assert_eq!(
        reply.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    assert_eq!(reply.choices[1].finish_reason, None);
    let calls = reply.choices[0].delta.tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].index, Some(0));
    assert_eq!(calls[1].index, Some(5));
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(calls[1].kind.as_deref(), Some("function"));
}

#[test]
fn stream_chunk_drops_the_service_tier() {
    let chunk =
        parsed(r#"{"id": "c", "created": 1, "model": "m", "choices": [], "service_tier": "auto"}"#);
    assert_eq!(chunk_parser(&chunk).unwrap().service_tier, None);
}

#[test]
fn stream_chunk_drops_a_null_usage() {
    let chunk = parsed(r#"{"id": "c", "created": 1, "model": "m", "choices": [], "usage": null}"#);
    assert_eq!(chunk_parser(&chunk).unwrap().usage, None);
}

#[test]
fn stream_chunk_clears_a_reasoning_content_without_a_reasoning_key() {
    let chunk = parsed(
        r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": {"content": "x", "reasoning_content": "old"}}]}"#,
    );
    let reply = chunk_parser(&chunk).unwrap();
    assert_eq!(reply.choices[0].delta.reasoning_content, None);
}

#[test]
fn stream_chunk_overwrites_an_existing_reasoning_content_with_null() {
    let chunk = parsed(
        r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": {"reasoning": null, "reasoning_content": "old"}}]}"#,
    );
    let reply = chunk_parser(&chunk).unwrap();
    assert_eq!(reply.choices[0].delta.reasoning_content, None);
    assert!(!reply.choices[0].delta.passthrough.contains_key("reasoning"));
}

#[rstest::rstest]
#[case::above_a_status(70000)]
#[case::negative(-5)]
fn stream_chunk_carries_an_error_code_outside_http_statuses(#[case] code: i64) {
    let chunk = parsed(&format!(
        r#"{{"error": {{"message": "boom", "code": {code}}}}}"#
    ));
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, code);
}

#[rstest::rstest]
#[case::string_code(json!("429"))]
#[case::float_code(json!(429.5))]
#[case::null_code(Value::Null)]
fn stream_chunk_names_a_non_integer_error_code(#[case] code: Value) {
    let chunk = parsed(&format!(
        r#"{{"error": {{"message": "boom", "code": {code}}}}}"#
    ));
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert!(error.message.starts_with("KeyError: 'code'"));
}

#[test]
fn stream_chunk_reports_the_openrouter_error() {
    let chunk = parsed(
        r#"{"error": {"message": "rate limited", "code": 429, "metadata": {"headers": {"Retry-After": "5"}, "provider_name": "Anthropic"}, "user_id": "u1"}}"#,
    );
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, 429);
    assert_eq!(error.headers.get("Retry-After"), Some(&String::from("5")));
    assert!(
        error
            .message
            .starts_with("Message: rate limited, Metadata: ")
    );
    assert!(error.message.ends_with(", User ID: u1"));
}

#[test]
fn stream_chunk_error_defaults_metadata_and_user() {
    let chunk = parsed(r#"{"error": {"message": "boom", "code": 500}}"#);
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert!(error.message.contains("Metadata: {}"));
    assert!(error.message.ends_with("User ID: "));
}

#[rstest::rstest]
#[case::no_id(parsed(r#"{"created": 1, "model": "m", "choices": []}"#), "id")]
#[case::no_created(parsed(r#"{"id": "c", "model": "m", "choices": []}"#), "created")]
#[case::no_model(parsed(r#"{"id": "c", "created": 1, "choices": []}"#), "model")]
#[case::no_choices(parsed(r#"{"id": "c", "created": 1, "model": "m"}"#), "choices")]
#[case::no_delta(
    parsed(r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0}]}"#),
    "delta"
)]
#[case::delta_not_object(
    parsed(r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": "x"}]}"#),
    "delta"
)]
#[case::choices_before_id(parsed("{}"), "choices")]
#[case::delta_before_id(parsed(r#"{"choices": [{"index": 0}]}"#), "delta")]
#[case::no_error_message(parsed(r#"{"error": {"code": 429}}"#), "message")]
#[case::no_error_code(parsed(r#"{"error": {"message": "boom"}}"#), "code")]
fn stream_chunk_missing_keys_give_the_key_error(#[case] chunk: Value, #[case] key: &str) {
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert!(error.message.starts_with(&format!("KeyError: '{key}'")));
    assert_eq!(
        error.headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );
}

#[test]
fn stream_chunk_rejects_a_wrong_shaped_reasoning() {
    let chunk = parsed(
        r#"{"id": "c", "created": 1, "model": "m", "choices": [{"index": 0, "delta": {"reasoning": 3}}]}"#,
    );
    let error = chunk_parser(&chunk).unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(
        error
            .message
            .starts_with("Got unexpected response from OpenRouter:")
    );
}

#[test]
fn error_class_carries_message_status_and_headers() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req"))]);
    let error = get_error_class("failed", 429, headers.clone());
    assert_eq!(error.message, "failed");
    assert_eq!(error.status_code, 429);
    assert_eq!(error.headers, headers);
    assert_eq!(error.to_string(), "failed");
}
