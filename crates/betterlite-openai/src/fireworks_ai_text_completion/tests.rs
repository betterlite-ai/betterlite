#![cfg(test)]

use std::future::{Future, ready};

use serde_json::{Map, Value, json};

use super::FireworksCompletionError;
use super::extra_body::{STRIP_PARAMS, map_extra_body_params};
use super::mapping::map_openai_params;
use super::request::transform_completion_request;
use super::supported::SUPPORTED_OPENAI_PARAMS;
use crate::fireworks_ai_chat_params::{FireworksCapabilities, NIM_VLLM_STRIP_PARAMS};

struct Capabilities {
    reasoning: bool,
}

impl FireworksCapabilities for Capabilities {
    fn supports_function_calling(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_tool_choice(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_reasoning(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.reasoning)
    }
}

fn plain() -> Capabilities {
    Capabilities { reasoning: false }
}

fn reasoner() -> Capabilities {
    Capabilities { reasoning: true }
}

fn params(text: &str) -> Map<String, Value> {
    let value: Value = serde_json::from_str(text).unwrap();
    match value {
        Value::Object(fields) => fields,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Map::new()
        }
    }
}

fn message(content: &str) -> Value {
    json!({"role": "user", "content": serde_json::from_str::<Value>(content).unwrap()})
}

#[test]
fn the_supported_list_is_litellms_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "logprobs",
            "echo",
            "temperature",
            "top_p",
            "top_k",
            "frequency_penalty",
            "presence_penalty",
            "n",
            "stop",
            "response_format",
            "stream",
            "user",
        ]
    );
}

#[test]
fn mapping_copies_supported_keys_and_keeps_null_values() {
    let non_default = [
        (String::from("temperature"), json!(0.5)),
        (String::from("stop"), Value::Null),
        (String::from("tools"), json!([])),
        (String::from("user"), json!("caller")),
        (String::from("n"), json!(2)),
    ];
    let mut optional = params(r#"{"temperature": 1, "presence_penalty": 1}"#);
    map_openai_params(&non_default, &mut optional);
    assert_eq!(
        optional,
        params(
            r#"{"temperature": 0.5, "presence_penalty": 1, "stop": null, "user": "caller", "n": 2}"#
        )
    );
}

#[test]
fn the_strip_list_is_the_nim_list_plus_two_prompt_keys() {
    let mut expected: Vec<&str> = NIM_VLLM_STRIP_PARAMS.to_vec();
    expected.push("truncate_prompt_tokens");
    expected.push("prompt_truncate_len");
    expected.sort_unstable();
    assert_eq!(STRIP_PARAMS.as_slice(), expected.as_slice());
}

#[tokio::test]
async fn a_map_extra_body_is_copied_without_touching_the_callers_params() {
    let optional = params(r#"{"temperature": 0.5, "extra_body": {"custom": {"kept": true}}}"#);
    let mapped = map_extra_body_params(&mut plain(), "model", &optional).await;
    assert_eq!(
        mapped,
        params(r#"{"temperature": 0.5, "extra_body": {"custom": {"kept": true}}}"#)
    );
    assert_eq!(
        optional,
        params(r#"{"temperature": 0.5, "extra_body": {"custom": {"kept": true}}}"#)
    );
}

#[tokio::test]
async fn a_non_map_extra_body_is_dropped() {
    let text = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": "plain", "temperature": 1}"#),
    )
    .await;
    assert_eq!(text, params(r#"{"temperature": 1}"#));
    let null = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": null, "temperature": 0}"#),
    )
    .await;
    assert_eq!(null, params(r#"{"temperature": 0}"#));
}

#[tokio::test]
async fn every_stripped_key_leaves_the_body() {
    let mut body: Map<String, Value> = STRIP_PARAMS
        .iter()
        .map(|key| (String::from(*key), json!(1)))
        .collect();
    body.insert(String::from("custom"), json!("kept"));
    let mut optional = Map::new();
    optional.insert(String::from("extra_body"), Value::Object(body));
    let mapped = map_extra_body_params(&mut plain(), "model", &optional).await;
    assert_eq!(mapped, params(r#"{"extra_body": {"custom": "kept"}}"#));
}

#[tokio::test]
async fn an_empty_body_adds_no_extra_body_key() {
    let empty =
        map_extra_body_params(&mut plain(), "model", &params(r#"{"extra_body": {}}"#)).await;
    assert_eq!(empty, Map::new());
    let absent = map_extra_body_params(&mut plain(), "model", &params(r#"{"top_p": 1}"#)).await;
    assert_eq!(absent, params(r#"{"top_p": 1}"#));
}

#[tokio::test]
async fn the_move_step_overrides_body_values_and_skips_null_values() {
    let overridden = map_extra_body_params(
        &mut plain(),
        "model",
        &params(
            r#"{"response_format": {"type": "json_object"}, "extra_body": {"response_format": {"type": "text"}, "custom": 2}}"#,
        ),
    )
    .await;
    assert_eq!(
        overridden,
        params(r#"{"extra_body": {"response_format": {"type": "json_object"}, "custom": 2}}"#)
    );
    let null_top_level = map_extra_body_params(
        &mut plain(),
        "model",
        &params(
            r#"{"reasoning_effort": null, "thinking": {"type": "enabled"}, "extra_body": {"reasoning_effort": "low", "thinking": "old"}}"#,
        ),
    )
    .await;
    assert_eq!(
        null_top_level,
        params(r#"{"extra_body": {"reasoning_effort": "low", "thinking": {"type": "enabled"}}}"#)
    );
}

#[tokio::test]
async fn the_top_level_native_params_move_into_extra_body() {
    let mapped = map_extra_body_params(
        &mut plain(),
        "model",
        &params(
            r#"{"response_format": {"type": "text"}, "reasoning_effort": "high", "thinking": {"type": "enabled"}, "temperature": 0}"#,
        ),
    )
    .await;
    assert_eq!(
        mapped,
        params(
            r#"{"temperature": 0, "extra_body": {"response_format": {"type": "text"}, "reasoning_effort": "high", "thinking": {"type": "enabled"}}}"#,
        )
    );
}

#[tokio::test]
async fn a_null_native_param_stays_on_the_top_level_only_through_the_body() {
    let mapped = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"response_format": null, "extra_body": {"custom": 1}}"#),
    )
    .await;
    assert_eq!(mapped, params(r#"{"extra_body": {"custom": 1}}"#));
}

#[tokio::test]
async fn template_kwargs_become_reasoning_effort_for_a_reasoning_model() {
    let mapped = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": {"reasoning_budget": 5}}}"#),
    )
    .await;
    assert_eq!(mapped, params(r#"{"extra_body": {"reasoning_effort": 5}}"#));
    let none = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": {"enable_thinking": false}}}"#),
    )
    .await;
    assert_eq!(
        none,
        params(r#"{"extra_body": {"reasoning_effort": "none"}}"#)
    );
    let low = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": {"low_effort": true, "unknown": 1}}}"#),
    )
    .await;
    assert_eq!(
        low,
        params(r#"{"extra_body": {"reasoning_effort": "low"}}"#)
    );
}

#[tokio::test]
async fn template_kwargs_are_dropped_for_a_model_without_reasoning() {
    let mapped = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": {"reasoning_budget": 5}}}"#),
    )
    .await;
    assert_eq!(mapped, Map::new());
}

#[tokio::test]
async fn template_kwargs_without_an_effort_are_removed() {
    let mapped = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": {"unknown": 1}, "custom": 2}}"#),
    )
    .await;
    assert_eq!(mapped, params(r#"{"extra_body": {"custom": 2}}"#));
}

#[tokio::test]
async fn a_non_map_or_null_chat_template_kwargs_is_handled() {
    let text = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": "plain", "custom": 1}}"#),
    )
    .await;
    assert_eq!(text, params(r#"{"extra_body": {"custom": 1}}"#));
    let null = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(r#"{"extra_body": {"chat_template_kwargs": null}}"#),
    )
    .await;
    assert_eq!(
        null,
        params(r#"{"extra_body": {"chat_template_kwargs": null}}"#)
    );
}

#[tokio::test]
async fn an_explicit_reasoning_setting_blocks_the_template_kwargs() {
    let top_level = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(
            r#"{"reasoning_effort": "high", "extra_body": {"chat_template_kwargs": {"low_effort": true}}}"#,
        ),
    )
    .await;
    assert_eq!(
        top_level,
        params(r#"{"extra_body": {"reasoning_effort": "high"}}"#)
    );
    let null_top_level = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(
            r#"{"reasoning_effort": null, "extra_body": {"chat_template_kwargs": {"low_effort": true}}}"#,
        ),
    )
    .await;
    assert_eq!(null_top_level, Map::new());
    let in_body = map_extra_body_params(
        &mut reasoner(),
        "model",
        &params(
            r#"{"extra_body": {"thinking": "enabled", "chat_template_kwargs": {"low_effort": true}}}"#,
        ),
    )
    .await;
    assert_eq!(
        in_body,
        params(r#"{"extra_body": {"thinking": "enabled"}}"#)
    );
}

#[tokio::test]
async fn guided_decoding_becomes_a_response_format_in_the_body() {
    let schema = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": {"guided_json": {"type": "object"}, "keep": 2}}"#),
    )
    .await;
    assert_eq!(
        schema,
        params(
            r#"{"extra_body": {"keep": 2, "response_format": {"type": "json_schema", "json_schema": {"name": "response", "schema": {"type": "object"}}}}}"#,
        )
    );
    let grammar = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": {"guided_choice": ["a"], "guided_grammar": "g"}}"#),
    )
    .await;
    assert_eq!(
        grammar,
        params(r#"{"extra_body": {"response_format": {"type": "grammar", "grammar": "g"}}}"#)
    );
    let choice = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": {"guided_choice": ["a"], "guided_json": null}}"#),
    )
    .await;
    assert_eq!(
        choice,
        params(
            r#"{"extra_body": {"response_format": {"type": "json_schema", "json_schema": {"name": "choice", "schema": {"type": "string", "enum": ["a"]}}}}}"#,
        )
    );
}

#[tokio::test]
async fn an_explicit_response_format_wins_and_the_guided_keys_go() {
    let mapped = map_extra_body_params(
        &mut plain(),
        "model",
        &params(
            r#"{"response_format": {"type": "json_object"}, "extra_body": {"guided_json": {"type": "object"}, "custom": 1}}"#,
        ),
    )
    .await;
    assert_eq!(
        mapped,
        params(r#"{"extra_body": {"custom": 1, "response_format": {"type": "json_object"}}}"#)
    );
}

#[tokio::test]
async fn null_guided_keys_produce_no_response_format() {
    let mapped = map_extra_body_params(
        &mut plain(),
        "model",
        &params(r#"{"extra_body": {"guided_json": null, "guided_grammar": null, "custom": 1}}"#),
    )
    .await;
    assert_eq!(mapped, params(r#"{"extra_body": {"custom": 1}}"#));
}

#[tokio::test]
async fn the_request_names_the_resource_and_projects_one_message() {
    let request = transform_completion_request(
        &mut plain(),
        "llama-v3p1-8b-instruct",
        &[message(r#""hello""#)],
        &Map::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        request,
        params(
            r#"{"model": "accounts/fireworks/models/llama-v3p1-8b-instruct", "prompt": "hello"}"#,
        )
    );
}

#[tokio::test]
async fn an_existing_resource_name_is_kept() {
    let request = transform_completion_request(
        &mut plain(),
        "accounts/fireworks/models/x",
        &[message(r#""hi""#)],
        &Map::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        request.get("model"),
        Some(&json!("accounts/fireworks/models/x"))
    );
}

#[tokio::test]
async fn several_messages_become_a_prompt_list() {
    let request = transform_completion_request(
        &mut plain(),
        "model",
        &[
            message(r#""a""#),
            message(r#"[{"type": "text", "text": "b"}]"#),
        ],
        &Map::new(),
    )
    .await
    .unwrap();
    assert_eq!(request.get("prompt"), Some(&json!(["a", "b"])));
}

#[tokio::test]
async fn a_single_token_message_keeps_its_tokens() {
    let request =
        transform_completion_request(&mut plain(), "model", &[message("[1, 2]")], &Map::new())
            .await
            .unwrap();
    assert_eq!(request.get("prompt"), Some(&json!([1, 2])));
}

#[tokio::test]
async fn the_optional_params_override_the_model_and_prompt() {
    let request = transform_completion_request(
        &mut plain(),
        "model",
        &[message(r#""hello""#)],
        &params(r#"{"model": "other", "prompt": "other prompt", "temperature": 0.5}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        request,
        params(r#"{"model": "other", "prompt": "other prompt", "temperature": 0.5}"#)
    );
}

#[tokio::test]
async fn the_request_carries_the_translated_extra_body() {
    let request = transform_completion_request(
        &mut plain(),
        "model",
        &[message(r#""hello""#)],
        &params(
            r#"{"temperature": 0.5, "extra_body": {"guided_json": {"type": "object"}, "custom": true}}"#,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        request,
        params(
            r#"{"model": "accounts/fireworks/models/model", "prompt": "hello", "temperature": 0.5, "extra_body": {"custom": true, "response_format": {"type": "json_schema", "json_schema": {"name": "response", "schema": {"type": "object"}}}}}"#,
        )
    );
}

#[tokio::test]
async fn a_malformed_message_fails_the_prompt_projection() {
    let malformed = json!({"content": "no role"});
    let error = transform_completion_request(&mut plain(), "model", &[malformed], &Map::new())
        .await
        .unwrap_err();
    assert!(matches!(error, FireworksCompletionError::Prompt(_)));
    assert_eq!(error.status_code(), None);
}
