#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate};
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use http::HeaderMap;
use rstest::rstest;

use super::{
    ReplicateBody, ReplicateDefaults, RequestContext, RequestInput, get_complete_url,
    model_to_version_id, transform_request, validate_environment,
};
use crate::replicate::error::ReplicateFailure;

fn params(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn parse_messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn parse_defaults(wire: &str) -> ReplicateDefaults {
    serde_json::from_str(wire).unwrap()
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn custom_prompts() -> BTreeMap<String, CustomPromptTemplate> {
    let mut roles = BTreeMap::new();
    roles.insert(
        String::from("user"),
        RoleTemplate {
            pre_message: String::from("U:"),
            post_message: String::from("\n"),
        },
    );
    let template = CustomPromptTemplate {
        roles,
        initial_prompt_value: String::from("I"),
        final_prompt_value: String::from("F"),
        bos_token: String::from("B"),
        eos_token: String::from("E"),
    };
    BTreeMap::from([(String::from("x/y:z"), template)])
}

async fn request(
    model: &str,
    wire_messages: &str,
    options: &str,
    defaults_wire: &str,
    custom: &BTreeMap<String, CustomPromptTemplate>,
) -> Result<(ReplicateBody, Vec<Message>, Object), ReplicateFailure> {
    let prompts = factory();
    let mut messages = parse_messages(wire_messages);
    let mut optional_params = params(options);
    let defaults = parse_defaults(defaults_wire);
    let body = transform_request(
        RequestInput {
            model,
            messages: &mut messages,
            optional_params: &mut optional_params,
            defaults: &defaults,
            custom_prompts: custom,
        },
        &RequestContext {
            prompts: &prompts,
            now: now(),
        },
    )
    .await?;
    Ok((body, messages, optional_params))
}

#[rstest]
#[case::plain("owner/model", "owner/model")]
#[case::versioned("owner/model:abc", "abc")]
#[case::three_parts("a:b:c", "b")]
#[case::empty_version("owner/model:", "")]
fn version_id_is_the_second_colon_part(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(model_to_version_id(model), expected);
}

#[rstest]
#[case::model(
    "owner/model",
    "https://api.replicate.com/v1/models/owner/model/predictions"
)]
#[case::versioned(
    "owner/model:abc",
    "https://api.replicate.com/v1/models/abc/predictions"
)]
#[case::three_parts("a:b:c", "https://api.replicate.com/v1/models/b/predictions")]
#[case::deployment(
    "owner/deployments/xyz",
    "https://api.replicate.com/v1/deployments/owner/xyz/predictions"
)]
#[case::deployment_prefix(
    "deployments/a/b",
    "https://api.replicate.com/v1/deployments/a/b/predictions"
)]
fn complete_url_ignores_the_base_and_follows_the_model_shape(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(get_complete_url(model), expected);
}

#[test]
fn environment_headers_drop_the_incoming_headers() {
    let mut incoming = HeaderMap::new();
    incoming.insert("authorization", "Bearer caller".parse().unwrap());
    incoming.insert("x-extra", "1".parse().unwrap());
    let headers = validate_environment(&incoming, Some("key")).unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers.get("authorization").unwrap(), "Token key");
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
}

#[test]
fn a_missing_api_key_renders_as_token_none() {
    let headers = validate_environment(&HeaderMap::new(), None).unwrap();
    assert_eq!(headers.get("authorization").unwrap(), "Token None");
}

#[tokio::test]
async fn a_plain_request_carries_the_prompt_and_the_options() {
    let (body, messages, options) = request(
        "meta-llama/llama-2-70b-chat",
        r#"[{"role":"user","content":"hi"}]"#,
        r#"{"temperature":0.2}"#,
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        body.input,
        params(r#"{"prompt":"<s>[INST] hi [/INST]\n","temperature":0.2}"#)
    );
    assert_eq!(body.version, None);
    assert_eq!(
        messages,
        parse_messages(r#"[{"role":"user","content":"hi"}]"#)
    );
    assert_eq!(options, params(r#"{"temperature":0.2}"#));
}

#[tokio::test]
async fn the_first_system_message_becomes_the_system_prompt() {
    let (body, remaining, options) = request(
        "meta-llama/llama-2-70b-chat",
        r#"[{"role":"system","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]},{"role":"user","content":"hi"}]"#,
        r#"{"supports_system_prompt":true}"#,
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        body.input,
        params(r#"{"prompt":"<s>[INST] hi [/INST]\n","system_prompt":"ab"}"#)
    );
    assert_eq!(
        remaining,
        parse_messages(r#"[{"role":"user","content":"hi"}]"#)
    );
    assert_eq!(options, params("{}"));
}

#[tokio::test]
async fn a_false_system_prompt_flag_keeps_the_message_in_the_prompt() {
    let (body, remaining, options) = request(
        "meta-llama/llama-2-70b-chat",
        r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#,
        r#"{"supports_system_prompt":false}"#,
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        body.input,
        params(r#"{"prompt":"<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\n"}"#)
    );
    assert_eq!(remaining.len(), 2);
    assert_eq!(options, params("{}"));
}

#[tokio::test]
async fn defaults_fill_the_options_only_where_the_call_has_no_key() {
    let (body, _, options) = request(
        "meta-llama/llama-2-70b-chat",
        r#"[{"role":"user","content":"u"}]"#,
        r#"{"temperature":9}"#,
        r#"{"system_prompt":"s","max_new_tokens":7,"temperature":1,"debug":false}"#,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        body.input,
        params(
            r#"{"prompt":"<s>[INST] u [/INST]\n","temperature":9,"system_prompt":"s","max_new_tokens":7,"debug":false}"#
        )
    );
    assert_eq!(
        options,
        params(r#"{"temperature":9,"system_prompt":"s","max_new_tokens":7,"debug":false}"#)
    );
}

#[tokio::test]
async fn a_custom_prompt_entry_is_used_for_the_full_model_string() {
    let (body, _, _) = request(
        "x/y:z",
        r#"[{"role":"user","content":"u"}]"#,
        "{}",
        "{}",
        &custom_prompts(),
    )
    .await
    .unwrap();
    assert_eq!(body.input, params(r#"{"prompt":"BIU:u\nF"}"#));
}

#[tokio::test]
async fn a_custom_prompt_entry_keyed_by_the_short_name_is_not_found() {
    let (body, _, _) = request(
        "meta-llama/llama-2-70b-chat:abc",
        r#"[{"role":"user","content":"u"}]"#,
        "{}",
        "{}",
        &custom_prompts(),
    )
    .await
    .unwrap();
    assert_eq!(body.input, params(r#"{"prompt":"<s>[INST] u [/INST]\n"}"#));
}

#[tokio::test]
async fn a_prompt_that_is_not_a_string_fails_400() {
    let error = request(
        "tiiuae/falcon-7b",
        r#"[{"role":"user","content":"u"}]"#,
        "{}",
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(status(&error), Some(400));
    assert_eq!(
        message(&error),
        Some("LiteLLM Error - prompt is not a string - None")
    );
    assert_eq!(headers_of(&error), Some(HeaderMap::new()));
}

#[tokio::test]
async fn supports_system_prompt_must_be_a_boolean() {
    let error = request(
        "meta-llama/llama-2-70b-chat",
        r#"[{"role":"user","content":"u"}]"#,
        r#"{"supports_system_prompt":"yes"}"#,
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReplicateFailure::SupportsSystemPrompt));
}

#[tokio::test]
async fn a_long_hash_model_never_adds_a_version_key() {
    let hash = "a".repeat(64);
    let model = format!("meta/llama-2-70b-chat:{hash}");
    let (body, _, _) = request(
        &model,
        r#"[{"role":"user","content":"hi"}]"#,
        "{}",
        "{}",
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(body.version, None);
    assert_eq!(body.input, params(r#"{"prompt":"hi"}"#));
    let url = get_complete_url(&model);
    assert_eq!(
        url,
        format!("https://api.replicate.com/v1/models/{hash}/predictions")
    );
}

fn status(failure: &ReplicateFailure) -> Option<u16> {
    failure.api().map(|error| error.status_code)
}

fn headers_of(failure: &ReplicateFailure) -> Option<HeaderMap> {
    failure.api()?.headers.clone()
}

fn message(failure: &ReplicateFailure) -> Option<&str> {
    failure.api().map(|error| error.message.as_str())
}
