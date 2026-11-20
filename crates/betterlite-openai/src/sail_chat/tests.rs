#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "json! fixtures state the sail wire the transforms read, numbers included, and each test keeps to one scenario"
    )
)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_error::error::ErrorClass;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};

use super::{MapRequest, SailChatConfig, supported_params};
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, RefuseDownloads};
use crate::openai_base_config::{CatalogCapabilities, Object as ParamsObject};
use crate::sail_common::{PROVIDER, SailSettings};

const MODEL: &str = "llama-3-70b";
const PARENT_WITHOUT_REJECTED: [&str; 24] = [
    "max_tokens",
    "max_completion_tokens",
    "modalities",
    "prediction",
    "n",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "max_retries",
    "extra_headers",
    "parallel_tool_calls",
    "audio",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
    "response_format",
];

fn params(value: Value) -> ParamsObject {
    serde_json::from_value(value).unwrap()
}

fn object(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn catalog(models: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: models.iter().map(|model| String::from(*model)).collect(),
        text_models: BTreeSet::new(),
    }
}

fn empty_catalog() -> CatalogCapabilities {
    catalog(&[])
}

fn settings(global_drop_params: bool) -> SailSettings {
    SailSettings { global_drop_params }
}

fn config(global_drop_params: bool) -> SailChatConfig {
    SailChatConfig {
        settings: settings(global_drop_params),
    }
}

fn user_message() -> ChatMessage {
    serde_json::from_value(json!({"role": "user", "content": "hi"})).unwrap()
}

fn content<'request>(
    messages: &'request [ChatMessage],
    optional_params: &'request Object,
    environment: &'request BTreeMap<String, String>,
) -> ChatContent<'request> {
    ChatContent {
        model: MODEL,
        messages,
        optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment,
        downloads: &RefuseDownloads,
    }
}

fn mapped(
    non_default: Value,
    optional: Value,
    settings: &SailSettings,
    drop_params: bool,
) -> ParamsObject {
    let mut optional_params = params(optional);
    config(settings.global_drop_params)
        .map_openai_params(MapRequest {
            model: MODEL,
            non_default_params: &params(non_default),
            optional_params: &mut optional_params,
            catalog: &empty_catalog(),
            drop_params,
        })
        .unwrap();
    optional_params
}

fn transformed(optional: Value, settings: &SailSettings, drop_params: bool) -> Value {
    let messages = [user_message()];
    let optional_params = object(optional);
    let environment = BTreeMap::new();
    let body = config(settings.global_drop_params)
        .transform_request(
            &content(&messages, &optional_params, &environment),
            drop_params,
        )
        .unwrap();
    serde_json::to_value(body).unwrap()
}

#[test]
fn supported_params_drop_the_seven_rejected_names_and_append_two() {
    let supported = supported_params(MODEL, &empty_catalog());
    let mut expected = PARENT_WITHOUT_REJECTED.to_vec();
    expected.extend(["reasoning_effort", "user"]);
    assert_eq!(supported, expected);
}

#[test]
fn a_catalog_model_carries_user_once_before_the_appended_effort() {
    let supported = supported_params("gpt-4o", &catalog(&["gpt-4o"]));
    let tail = &supported[supported.len() - 3..];
    let expected_tail: &[&str] = &["response_format", "user", "reasoning_effort"];
    assert_eq!(tail, expected_tail);
    assert_eq!(supported.iter().filter(|name| **name == "user").count(), 1);
}

#[test]
fn the_two_gpt_models_without_response_format_keep_the_two_appended_names_last() {
    let supported = supported_params("gpt-4", &empty_catalog());
    assert_eq!(supported.last(), Some(&"user"));
    assert_eq!(
        supported.get(supported.len() - 2),
        Some(&"reasoning_effort")
    );
    assert!(!supported.contains(&"response_format"));
    let short = supported_params("gpt-3.5-turbo-16k", &empty_catalog());
    assert!(!short.contains(&"response_format"));
}

#[test]
fn the_rejected_names_never_survive_a_catalog_list() {
    let supported = supported_params("gpt-4o", &catalog(&["gpt-4o"]));
    let rejected = [
        "stop",
        "seed",
        "frequency_penalty",
        "presence_penalty",
        "logit_bias",
        "logprobs",
        "top_logprobs",
    ];
    assert!(rejected.iter().all(|name| !supported.contains(name)));
}

#[test]
fn a_bad_service_tier_fails_the_mapping_even_though_the_window_is_unused() {
    let mut optional_params = params(json!({}));
    let error = config(false)
        .map_openai_params(MapRequest {
            model: MODEL,
            non_default_params: &params(json!({"service_tier": "flash"})),
            optional_params: &mut optional_params,
            catalog: &empty_catalog(),
            drop_params: false,
        })
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert!(optional_params.is_empty());
}

#[test]
fn dropping_through_the_argument_or_the_setting_lets_the_bad_tier_through() {
    let by_argument = mapped(
        json!({"service_tier": "flash"}),
        json!({}),
        &settings(false),
        true,
    );
    assert_eq!(
        serde_json::to_value(&by_argument).unwrap(),
        json!({"service_tier": "flash"})
    );
    let by_setting = mapped(
        json!({"service_tier": "flash"}),
        json!({}),
        &settings(true),
        false,
    );
    assert_eq!(
        serde_json::to_value(&by_setting).unwrap(),
        json!({"service_tier": "flash"})
    );
}

#[test]
fn the_mapping_copies_only_the_supported_names() {
    let optional = mapped(
        json!({
            "temperature": 0.2,
            "reasoning_effort": "low",
            "user": "u-1",
            "stop": ["x"],
            "seed": 7,
            "thinking": "on",
        }),
        json!({}),
        &settings(false),
        false,
    );
    assert_eq!(
        serde_json::to_value(&optional).unwrap(),
        json!({"temperature": 0.2, "reasoning_effort": "low", "user": "u-1"})
    );
}

#[test]
fn the_mapping_keeps_an_existing_value_until_a_requested_one_replaces_it() {
    let optional = mapped(
        json!({"temperature": 0.9}),
        json!({"temperature": 0.2, "top_p": 0.5}),
        &settings(false),
        false,
    );
    assert_eq!(
        serde_json::to_value(&optional).unwrap(),
        json!({"temperature": 0.9, "top_p": 0.5})
    );
}

#[test]
fn a_null_tier_maps_without_an_error() {
    let optional = mapped(
        json!({"service_tier": null}),
        json!({}),
        &settings(false),
        false,
    );
    assert_eq!(
        serde_json::to_value(&optional).unwrap(),
        json!({"service_tier": null})
    );
}

#[test]
fn an_absent_tier_maps_without_an_error() {
    let optional = mapped(
        json!({"temperature": 0.2}),
        json!({}),
        &settings(false),
        false,
    );
    assert_eq!(
        serde_json::to_value(&optional).unwrap(),
        json!({"temperature": 0.2})
    );
}

#[test]
fn the_request_moves_the_tier_into_the_metadata_window() {
    let body = transformed(
        json!({"service_tier": "flex", "temperature": 0.2, "max_retries": 3}),
        &settings(false),
        false,
    );
    assert_eq!(
        body,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
            "metadata": {"completion_window": "flex"},
        })
    );
}

#[test]
fn the_request_merges_a_mapping_extra_body_over_the_window_metadata() {
    let body = transformed(
        json!({
            "service_tier": "balanced",
            "extra_body": {"user": "u-2", "metadata": {"user_id": "u-1"}},
        }),
        &settings(false),
        false,
    );
    assert_eq!(
        body,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
            "metadata": {"completion_window": "balanced"},
            "extra_body": {
                "user": "u-2",
                "metadata": {"user_id": "u-1", "completion_window": "balanced"},
            },
        })
    );
}

#[test]
fn the_request_drops_a_non_mapping_extra_body() {
    let body = transformed(
        json!({"extra_body": "raw", "temperature": 0.2}),
        &settings(false),
        false,
    );
    assert_eq!(
        body,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
        })
    );
}

#[test]
fn the_request_rejects_a_bad_tier_with_the_unsupported_class() {
    let messages = [user_message()];
    let optional_params = object(json!({"service_tier": "flash"}));
    let environment = BTreeMap::new();
    let error = config(false)
        .transform_request(&content(&messages, &optional_params, &environment), false)
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
    let Some(ErrorClass::UnsupportedParams(context)) = error.class() else {
        panic!("a sail tier error is an unsupported params error");
    };
    assert_eq!(context.provider.as_deref(), Some(PROVIDER));
    assert_eq!(context.model.as_deref(), Some(MODEL));
}

#[test]
fn the_request_drops_a_bad_tier_when_the_call_drops_params() {
    let body = transformed(
        json!({"service_tier": "flash", "temperature": 0.2}),
        &settings(false),
        true,
    );
    assert_eq!(
        body,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
        })
    );
}

#[test]
fn the_request_drops_a_bad_tier_through_the_global_setting() {
    let body = transformed(json!({"service_tier": "flash"}), &settings(true), false);
    assert_eq!(
        body,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
        })
    );
}

#[test]
fn the_request_rejects_a_caller_window_and_accepts_it_when_dropping() {
    let messages = [user_message()];
    let optional_params = object(json!({"metadata": {"completion_window": "asap"}}));
    let environment = BTreeMap::new();
    let error = config(false)
        .transform_request(&content(&messages, &optional_params, &environment), false)
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
    let dropped = transformed(
        json!({"metadata": {"completion_window": "asap"}}),
        &settings(false),
        true,
    );
    assert_eq!(
        dropped,
        json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "hi"}],
        })
    );
}

#[test]
fn the_request_strips_cache_control_as_the_sail_provider() {
    let cached: ChatMessage = serde_json::from_value(json!({
        "role": "user",
        "content": "hi",
        "cache_control": {"type": "ephemeral"},
    }))
    .unwrap();
    let messages = [cached];
    let optional_params = object(json!({}));
    let environment = BTreeMap::new();
    let mut openai = content(&messages, &optional_params, &environment);
    openai.provider = Some("openai");
    openai.api_base = Some("https://sail-gateway.test/v1");
    let body = config(false).transform_request(&openai, false).unwrap();
    let Some(Json::List(rendered)) = body.get("messages") else {
        panic!("the transformed request keeps its messages");
    };
    let Some(Json::Object(message)) = rendered.first() else {
        panic!("the user message survives the transform");
    };
    assert!(!message.contains_key("cache_control"));
    assert_eq!(
        message.get("content"),
        Some(&Json::Text(String::from("hi")))
    );
}

#[test]
fn transform_extra_body_rejects_a_tier_and_a_caller_window() {
    let request = object(json!({"metadata": {"completion_window": "flex"}}));
    let tier = config(false)
        .transform_extra_body(
            &object(json!({"service_tier": "flex"})),
            &request,
            MODEL,
            false,
        )
        .unwrap_err();
    assert_eq!(tier.status_code(), 400);
    let window = config(false)
        .transform_extra_body(
            &object(json!({"metadata": {"completion_window": "asap"}})),
            &request,
            MODEL,
            false,
        )
        .unwrap_err();
    assert_eq!(window.status_code(), 400);
}

#[test]
fn transform_extra_body_merges_the_request_metadata_over_the_caller_one() {
    let request = object(json!({"metadata": {"completion_window": "flex"}}));
    let merged = config(false)
        .transform_extra_body(
            &object(json!({"user": "u", "metadata": {"trace": "t-1"}})),
            &request,
            MODEL,
            false,
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&merged).unwrap(),
        json!({"user": "u", "metadata": {"trace": "t-1", "completion_window": "flex"}})
    );
}

#[test]
fn transform_extra_body_drops_a_tier_through_the_argument_or_the_setting() {
    let request = object(json!({}));
    let extra = object(json!({"service_tier": "flex", "user": "u"}));
    let by_argument = config(false)
        .transform_extra_body(&extra, &request, MODEL, true)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&by_argument).unwrap(),
        json!({"user": "u"})
    );
    let by_setting = config(true)
        .transform_extra_body(&extra, &request, MODEL, false)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&by_setting).unwrap(),
        json!({"user": "u"})
    );
}
