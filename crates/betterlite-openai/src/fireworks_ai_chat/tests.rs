#![cfg(test)]

mod models;
mod replies;
mod requests;
mod status;
mod streams;

use std::collections::BTreeMap;

use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use uuid::Uuid;

use super::auth::{DEFAULT_API_BASE, FireworksChatEnv, provider_info, validate_environment};
use super::messages::transform_messages;
use super::request::{RequestInput, transform_request};
use super::tools::{function_names, transform_tools};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::fireworks_ai_common::{FireworksEnv, SessionParams};
use crate::openai_base_config::Headers;

fn secret(text: &str) -> SecretString {
    SecretString::from(String::from(text))
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn response_headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn prompt_json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON value")
}

fn message(text: &str) -> ChatMessage {
    serde_json::from_str(text).expect("a chat message")
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

fn fixed_clock() -> i64 {
    1_700_000_000
}

fn env_keys(
    fireworks: Option<&str>,
    ai: Option<&str>,
    joined: Option<&str>,
    token: Option<&str>,
) -> FireworksEnv {
    FireworksEnv {
        fireworks_api_key: fireworks.map(secret),
        fireworks_ai_api_key: ai.map(secret),
        fireworksai_api_key: joined.map(secret),
        fireworks_ai_token: token.map(secret),
    }
}

fn chat_env(env: FireworksEnv) -> FireworksChatEnv {
    FireworksChatEnv {
        keys: env,
        api_base: None,
        account_id: None,
    }
}

fn models_env(api_base: Option<&str>, account: Option<&str>) -> FireworksChatEnv {
    FireworksChatEnv {
        keys: FireworksEnv::default(),
        api_base: api_base.map(str::to_owned),
        account_id: account.map(str::to_owned),
    }
}

fn authorization(env: FireworksEnv, argument: Option<&SecretString>) -> String {
    let environment = chat_env(env);
    let validated = validate_environment(
        argument,
        &Headers::new(),
        &SessionParams::default(),
        &environment,
    )
    .expect("headers");
    validated
        .get("Authorization")
        .cloned()
        .expect("authorization")
}

fn model_fields(info: &str) -> ModelFields {
    let value: Value = serde_json::from_str(info).expect("model info");
    let raw = to_raw_value(&value).expect("raw JSON");
    ModelFields::parse(&raw).expect("model fields").0
}

fn model_catalog(entries: &[(&str, &str)]) -> Catalog {
    entries
        .iter()
        .map(|(name, info)| ((*name).to_owned(), model_fields(info)))
        .collect()
}

fn cleaned_messages(messages: &[ChatMessage], model: &str, catalog: &Catalog) -> Vec<Value> {
    transform_messages(messages, model, catalog)
        .expect("messages")
        .iter()
        .map(|entry| serde_json::to_value(entry).expect("message JSON"))
        .collect()
}

fn request_body(model: &str, params: &str) -> Object {
    let messages = vec![message(r#"{"role": "user", "content": "hi"}"#)];
    let optional_params = object(params);
    let environment = BTreeMap::new();
    transform_request(&RequestInput {
        model,
        messages: &messages,
        optional_params: &optional_params,
        catalog: &Catalog::new(),
        environment: &environment,
        downloads: &RefuseDownloads,
    })
    .expect("request body")
}

#[test]
fn validate_environment_sets_the_bearer_and_keeps_the_caller_headers() {
    let caller = header_map(&[("content-type", "text/plain"), ("x-extra", "1")]);
    let validated = validate_environment(
        Some(&secret("direct")),
        &caller,
        &SessionParams::default(),
        &chat_env(FireworksEnv::default()),
    )
    .expect("headers");
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer direct")
    );
    assert_eq!(
        validated.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert!(!validated.contains_key("Content-Type"));
    assert_eq!(validated.get("x-extra").map(String::as_str), Some("1"));
}

#[test]
fn validate_environment_adds_the_json_content_type() {
    let validated = validate_environment(
        Some(&secret("direct")),
        &Headers::new(),
        &SessionParams::default(),
        &chat_env(FireworksEnv::default()),
    )
    .expect("headers");
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn validate_environment_replaces_a_caller_authorization() {
    let validated = validate_environment(
        Some(&secret("direct")),
        &header_map(&[("Authorization", "Bearer caller")]),
        &SessionParams::default(),
        &chat_env(FireworksEnv::default()),
    )
    .expect("headers");
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer direct")
    );
}

#[test]
fn validate_environment_keeps_a_caller_content_type_header() {
    let validated = validate_environment(
        Some(&secret("direct")),
        &header_map(&[("Content-Type", "application/json; charset=utf-8")]),
        &SessionParams::default(),
        &chat_env(FireworksEnv::default()),
    )
    .expect("headers");
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json; charset=utf-8")
    );
}

#[test]
fn validate_environment_walks_the_key_chain() {
    let direct = secret("direct");
    assert_eq!(
        authorization(env_keys(None, None, None, None), Some(&direct)),
        "Bearer direct"
    );
    assert_eq!(
        authorization(env_keys(Some("first"), Some("second"), None, None), None),
        "Bearer first"
    );
    assert_eq!(
        authorization(env_keys(None, Some("ai"), None, None), None),
        "Bearer ai"
    );
    assert_eq!(
        authorization(env_keys(None, None, Some("joined"), None), None),
        "Bearer joined"
    );
    assert_eq!(
        authorization(env_keys(None, None, None, Some("token")), None),
        "Bearer token"
    );
}

#[test]
fn validate_environment_without_a_key_is_missing_api_key() {
    let empty = secret("");
    let error = validate_environment(
        Some(&empty),
        &Headers::new(),
        &SessionParams::default(),
        &chat_env(env_keys(Some(""), None, None, None)),
    )
    .expect_err("no key");
    assert_eq!(error.to_string(), "FIREWORKS_API_KEY is not set");
    assert_eq!(error.status_code(), None);
}

#[test]
fn validate_environment_pins_the_session_affinity_header() {
    let session = SessionParams {
        litellm_session_id: Some(String::from("session-1")),
        session_id: None,
        metadata: None,
    };
    let validated = validate_environment(
        Some(&secret("direct")),
        &Headers::new(),
        &session,
        &chat_env(FireworksEnv::default()),
    )
    .expect("headers");
    assert_eq!(
        validated.get("x-session-affinity").map(String::as_str),
        Some("session-1")
    );
}

#[test]
fn provider_info_picks_the_base() {
    let empty = chat_env(FireworksEnv::default());
    assert_eq!(provider_info(None, None, &empty).0, DEFAULT_API_BASE);
    assert_eq!(
        provider_info(Some("https://custom/v1"), None, &empty).0,
        "https://custom/v1"
    );
    assert_eq!(
        provider_info(None, None, &models_env(Some("https://env/"), None)).0,
        "https://env/"
    );
    assert_eq!(
        provider_info(None, None, &models_env(Some(""), None)).0,
        DEFAULT_API_BASE
    );
    assert_eq!(
        provider_info(
            Some("https://argument"),
            None,
            &models_env(Some("https://env"), None)
        )
        .0,
        "https://argument"
    );
    assert!(provider_info(None, None, &empty).1.is_none());
}

#[test]
fn provider_info_returns_the_resolved_key() {
    let environment = chat_env(env_keys(None, None, None, Some("token")));
    let (_, resolved) = provider_info(None, None, &environment);
    assert_eq!(
        resolved.map(|key| key.expose_secret().to_owned()),
        Some(String::from("token"))
    );
}

#[test]
fn transform_tools_removes_strict_and_unpacks_legacy_definitions() {
    let mut tools = prompt_json(
        r##"[
        {
            "type": "function",
            "function": {
                "name": "lookup",
                "strict": true,
                "parameters": {
                    "type": "object",
                    "properties": {"city": {"$ref": "#/definitions/City"}},
                    "definitions": {"City": {"type": "string"}}
                }
            }
        },
        {"type": "web_search"}
    ]"##,
    );
    transform_tools(&mut tools).expect("tools");
    let Json::List(entries) = &tools else {
        panic!("tools list");
    };
    let Json::Object(tool) = &entries[0] else {
        panic!("tool object");
    };
    let Json::Object(function) = &tool["function"] else {
        panic!("function object");
    };
    assert!(!function.contains_key("strict"));
    let parameters = function["parameters"].as_object().expect("parameters");
    assert!(!parameters.contains_key("definitions"));
    let properties = parameters["properties"].as_object().expect("properties");
    let city = properties["city"].as_object().expect("city");
    assert_eq!(city["type"], prompt_json(r#""string""#));
    assert_eq!(entries[1], prompt_json(r#"{"type": "web_search"}"#));
}

#[test]
fn transform_tools_rejects_tools_that_are_not_a_list() {
    let mut tools = prompt_json(r#"{"type": "function"}"#);
    let error = transform_tools(&mut tools).expect_err("not a list");
    assert_eq!(error.to_string(), "tools must be a list");
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn transform_tools_rejects_a_tool_that_is_not_an_object() {
    let mut tools = prompt_json(r#"["lookup"]"#);
    let error = transform_tools(&mut tools).expect_err("not an object");
    assert_eq!(
        error.to_string(),
        "tools[0] is not an object with a function"
    );
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn transform_tools_rejects_a_function_tool_without_a_function() {
    let mut tools = prompt_json(r#"[{"type": "function"}]"#);
    let error = transform_tools(&mut tools).expect_err("no function");
    assert_eq!(
        error.to_string(),
        "tools[0] is not an object with a function"
    );
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn function_names_skips_stringless_names_and_refuses_malformed_tools() {
    let tools = prompt_json(
        r#"[
        {"type": "function", "function": {"name": "lookup"}},
        {"type": "function", "function": {"name": 4}}
    ]"#,
    );
    assert_eq!(
        function_names(&tools).expect("names"),
        vec![String::from("lookup")]
    );
    assert!(function_names(&prompt_json(r#"[{"type": "function"}]"#)).is_none());
    assert!(function_names(&prompt_json(r#"{"tools": []}"#)).is_none());
    assert!(function_names(&Json::Null).is_none());
}

#[test]
fn message_cleanup_removes_cache_control_at_every_depth() {
    let messages = vec![
        message(
            r#"{
            "role": "user",
            "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}],
            "cache_control": {"type": "ephemeral"}
        }"#,
        ),
        message(
            r#"{
            "role": "assistant",
            "tool_calls": [{
                "id": "call-1",
                "type": "function",
                "function": {"name": "lookup", "arguments": "{}", "cache_control": {"type": "ephemeral"}}
            }]
        }"#,
        ),
    ];
    let cleaned = cleaned_messages(&messages, "models/llama", &Catalog::new());
    assert!(cleaned[0].get("cache_control").is_none());
    assert!(cleaned[0]["content"][0].get("cache_control").is_none());
    assert!(
        cleaned[1]["tool_calls"][0]["function"]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(cleaned[1]["tool_calls"][0]["id"], json!("call-1"));
}

#[test]
fn message_cleanup_removes_provider_specific_fields_and_thinking_blocks() {
    let messages = vec![message(
        r#"{
        "role": "assistant",
        "content": "answer",
        "provider_specific_fields": {"signature": "s"},
        "thinking_blocks": [{"type": "thinking", "thinking": "why"}]
    }"#,
    )];
    let cleaned = cleaned_messages(&messages, "models/llama", &Catalog::new());
    assert!(cleaned[0].get("provider_specific_fields").is_none());
    assert!(cleaned[0].get("thinking_blocks").is_none());
    assert_eq!(cleaned[0]["content"], json!("answer"));
}

#[test]
fn file_content_block_is_a_bad_request() {
    let messages = vec![message(
        r#"{
        "role": "user",
        "content": [{"type": "file", "file": {"file_data": "data:application/pdf;base64,AA=="}}]
    }"#,
    )];
    let error =
        transform_messages(&messages, "models/llama", &Catalog::new()).expect_err("file block");
    assert_eq!(error.status_code(), Some(400));
    assert!(
        error
            .to_string()
            .contains("does not support file content blocks")
    );
}

#[test]
fn file_content_block_in_a_non_user_message_passes_through() {
    let messages = vec![message(
        r#"{
        "role": "assistant",
        "content": [{"type": "file", "file": {"file_data": "data:application/pdf;base64,AA=="}}]
    }"#,
    )];
    let cleaned = cleaned_messages(&messages, "models/llama", &Catalog::new());
    assert_eq!(cleaned[0]["content"][0]["type"], json!("file"));
}

#[test]
fn image_url_block_is_refused_for_a_model_without_vision() {
    let messages = vec![message(
        r#"{
        "role": "user",
        "content": [{"type": "image_url", "image_url": {"url": "https://img/1.png"}}]
    }"#,
    )];
    let catalog = model_catalog(&[("fireworks_ai/llama", r#"{"supports_vision": false}"#)]);
    let error = transform_messages(&messages, "accounts/fireworks/models/llama", &catalog)
        .expect_err("image block");
    assert_eq!(error.status_code(), Some(400));
    assert!(
        error
            .to_string()
            .contains("accounts/fireworks/models/llama")
    );
}

#[test]
fn image_url_block_passes_for_a_vision_model() {
    let messages = vec![message(
        r#"{
        "role": "user",
        "content": [{"type": "image_url", "image_url": {"url": "https://img/1.png"}}]
    }"#,
    )];
    let catalog = model_catalog(&[("fireworks_ai/llama", r#"{"supports_vision": true}"#)]);
    let cleaned = cleaned_messages(&messages, "accounts/fireworks/models/llama", &catalog);
    assert_eq!(cleaned[0]["content"][0]["type"], json!("image_url"));
}

#[test]
fn image_url_block_passes_when_the_capability_is_unknown() {
    let messages = vec![message(
        r#"{
        "role": "user",
        "content": [{"type": "image_url", "image_url": {"url": "https://img/1.png"}}]
    }"#,
    )];
    let catalog = model_catalog(&[("fireworks_ai/other", r#"{"supports_vision": false}"#)]);
    let cleaned = cleaned_messages(&messages, "accounts/fireworks/models/llama", &catalog);
    assert_eq!(cleaned[0]["content"][0]["type"], json!("image_url"));
}

#[test]
fn image_url_block_in_a_non_user_message_passes_through() {
    let messages = vec![message(
        r#"{
        "role": "assistant",
        "content": [{"type": "image_url", "image_url": {"url": "https://img/1.png"}}]
    }"#,
    )];
    let catalog = model_catalog(&[("fireworks_ai/llama", r#"{"supports_vision": false}"#)]);
    let cleaned = cleaned_messages(&messages, "accounts/fireworks/models/llama", &catalog);
    assert_eq!(cleaned[0]["content"][0]["type"], json!("image_url"));
}
