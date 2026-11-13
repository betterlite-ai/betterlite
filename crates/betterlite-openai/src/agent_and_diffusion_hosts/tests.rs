#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::agent_host_settings::Settings;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::gradient::{
    GradientAiConfig, GradientError, GradientSettings, SERVERLESS_ENDPOINT, SUPPORTED_PARAMS,
};
use super::heroku::{HerokuChatConfig, HerokuError};
use super::inception_fim::{self, InceptionTextCompletionConfig};
use super::{AgentHost, HYPERBOLIC_PARAMS, INCEPTION_API_BASE, INCEPTION_PARAMS};
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContentError, MissingFileError, Mode};
use crate::openai_base_config::{Headers, Object};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .agent_host_settings
}

fn settings_from(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the test config loads")
    .sections
    .agent_host_settings
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn params(value: Value) -> IndexMap<String, Value> {
    serde_json::from_value(value).expect("a parameter map")
}

fn object(value: Value) -> Object {
    serde_json::from_value(value).expect("an object")
}

fn headers(value: Value) -> Headers {
    serde_json::from_value(value).expect("a header map")
}

#[test]
fn gradient_lists_both_token_limits_and_its_own_options() {
    assert_eq!(
        AgentHost::Gradient.supported_params(),
        Some(SUPPORTED_PARAMS)
    );
    assert_eq!(
        SUPPORTED_PARAMS,
        [
            "frequency_penalty",
            "max_tokens",
            "max_completion_tokens",
            "presence_penalty",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "k",
            "kb_filters",
            "filter_kb_content_by_query_metadata",
            "instruction_override",
            "include_functions_info",
            "include_retrieval_info",
            "include_guardrails_info",
            "provide_citations",
            "retrieval_method",
        ]
    );
}

#[test]
fn gradient_copies_supported_values_including_nulls() {
    let requested = params(json!({"temperature": null, "stop": ["a"], "k": 3_i32}));
    let mut existing = Object::new();
    GradientAiConfig::map_params(&requested, &mut existing, false)
        .expect("all three are supported");
    assert_eq!(
        Value::Object(existing),
        json!({"temperature": null, "stop": ["a"], "k": 3_i32})
    );
}

#[test]
fn gradient_rejects_an_unsupported_option_unless_dropped() {
    let requested = params(json!({"top_k": 1_i32}));
    let error = GradientAiConfig::map_params(&requested, &mut Object::new(), false)
        .expect_err("top_k is not supported");
    assert_eq!(
        error,
        GradientError::UnsupportedParam {
            name: String::from("top_k")
        }
    );
    assert_eq!(error.status_code(), Some(400));
    let mut existing = Object::new();
    GradientAiConfig::map_params(&requested, &mut existing, true).expect("drop_params skips it");
    assert!(existing.is_empty());
}

#[test]
fn gradient_keeps_both_token_limit_names() {
    let requested = params(json!({"max_completion_tokens": 5_i32, "max_tokens": 7_i32}));
    let mut existing = Object::new();
    GradientAiConfig::map_params(&requested, &mut existing, false).expect("both are supported");
    assert_eq!(
        Value::Object(existing),
        json!({"max_completion_tokens": 5_i32, "max_tokens": 7_i32})
    );
}

#[test]
fn gradient_settings_start_empty_and_keep_what_was_given() {
    assert!(GradientSettings::default().get_config().is_empty());
    let given: GradientSettings = serde_json::from_value(json!({
        "max_tokens": 3_i32,
        "max_completion_tokens": 9_i32,
        "k": 0_i32,
        "filter_kb_content_by_query_metadata": false,
        "include_guardrails_info": null
    }))
    .expect("a settings snapshot");
    assert_eq!(
        Value::Object(given.get_config()),
        json!({
            "max_tokens": 3_i32,
            "max_completion_tokens": 9_i32,
            "k": 0_i32,
            "filter_kb_content_by_query_metadata": false
        })
    );
}

#[rstest]
#[case::missing_key(None, None, None)]
#[case::environment_key(None, Some("env"), Some("env"))]
#[case::explicit_key(None, Some("explicit"), Some("explicit"))]
#[case::explicit_empty_falls_to_environment(Some(""), Some("env"), Some("env"))]
#[case::explicit_empty_with_empty_environment(Some(""), Some(""), Some(""))]
#[case::explicit_wins_over_environment(Some("explicit"), Some("env"), Some("explicit"))]
fn gradient_validation_falls_back_to_the_environment_key(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let entries: Vec<(&str, &str)> = environment
        .map(|value| vec![("GRADIENT_AI_API_KEY", value)])
        .unwrap_or_default();
    let loaded = settings(&entries);
    let key_override = explicit.map(secret);
    let validated = GradientAiConfig::validate_environment(&loaded, key_override.as_ref(), None);
    match expected {
        Some(wanted) => {
            let found = validated.expect("a key was found");
            assert_eq!(
                found.get("Authorization"),
                Some(&format!("Bearer {wanted}"))
            );
            assert_eq!(
                found.get("Content-Type").map(String::as_str),
                Some("application/json")
            );
        }
        None => assert_eq!(
            validated.expect_err("no key was found"),
            GradientError::MissingApiKey
        ),
    }
}

#[test]
fn gradient_validation_creates_headers_and_replaces_the_two_it_owns() {
    let loaded = settings(&[("GRADIENT_AI_API_KEY", "env")]);
    let given = headers(json!({"authorization": "mine", "Content-Type": "text/plain"}));
    let validated =
        GradientAiConfig::validate_environment(&loaded, Some(&secret("explicit")), Some(given))
            .expect("a key was found");
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(
        validated.get("authorization").map(String::as_str),
        Some("mine")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::custom_base(
    Some("https://custom.example"),
    None,
    "https://custom.example/api/v1/chat/completions"
)]
#[case::serverless_base_with_agent_environment(
    Some(SERVERLESS_ENDPOINT),
    Some("https://agent.example"),
    "https://agent.example/api/v1/chat/completions"
)]
#[case::serverless_base_alone(
    Some(SERVERLESS_ENDPOINT),
    None,
    "https://inference.do-ai.run/v1/chat/completions"
)]
#[case::agent_environment_alone(
    None,
    Some("https://agent.example"),
    "https://agent.example/api/v1/chat/completions"
)]
#[case::nothing_set(None, None, "https://inference.do-ai.run/v1/chat/completions")]
#[case::empty_base_and_agent_environment(
    Some(""),
    Some("https://agent.example"),
    "https://agent.example/api/v1/chat/completions"
)]
#[case::agent_environment_is_the_default(
    None,
    Some(SERVERLESS_ENDPOINT),
    "https://inference.do-ai.run/v1/chat/completions"
)]
#[case::bases_are_not_trimmed(
    Some("https://custom.example/"),
    None,
    "https://custom.example//api/v1/chat/completions"
)]
#[case::agent_endpoint_is_not_trimmed(
    None,
    Some("https://agent.example/"),
    "https://agent.example//api/v1/chat/completions"
)]
fn gradient_complete_url_reads_the_agent_environment_again(
    #[case] api_base: Option<&str>,
    #[case] agent_endpoint: Option<&str>,
    #[case] expected: &str,
) {
    let entries: Vec<(&str, &str)> = agent_endpoint
        .map(|value| vec![("GRADIENT_AI_AGENT_ENDPOINT", value)])
        .unwrap_or_default();
    let loaded = settings(&entries);
    assert_eq!(
        GradientAiConfig::get_complete_url(&loaded, api_base),
        expected
    );
}

#[rstest]
#[case::explicit_base(
    Some("https://explicit"),
    &[],
    Some("explicit-key"),
    Some("https://explicit"),
    Some("explicit-key")
)]
#[case::explicit_base_beats_agent_environment(
    Some("https://explicit"),
    &[("GRADIENT_AI_AGENT_ENDPOINT", "https://agent"), ("GRADIENT_AI_API_KEY", "env-key")],
    None,
    Some("https://explicit"),
    Some("env-key")
)]
#[case::agent_environment_default(
    None,
    &[("GRADIENT_AI_AGENT_ENDPOINT", "https://agent"), ("GRADIENT_AI_API_KEY", "env-key")],
    None,
    Some("https://agent"),
    Some("env-key")
)]
#[case::serverless_default(
    None,
    &[("GRADIENT_AI_API_KEY", "env-key")],
    None,
    Some(SERVERLESS_ENDPOINT),
    Some("env-key")
)]
#[case::empty_base_falls_to_agent_environment(
    Some(""),
    &[("GRADIENT_AI_AGENT_ENDPOINT", "https://agent"), ("GRADIENT_AI_API_KEY", "env-key")],
    None,
    Some("https://agent"),
    Some("env-key")
)]
#[case::empty_environment_values_fall_through(
    None,
    &[("GRADIENT_AI_AGENT_ENDPOINT", ""), ("GRADIENT_AI_API_KEY", "")],
    Some("explicit"),
    Some(SERVERLESS_ENDPOINT),
    Some("explicit")
)]
#[case::both_empty_environment_values(
    None,
    &[("GRADIENT_AI_AGENT_ENDPOINT", ""), ("GRADIENT_AI_API_KEY", "")],
    None,
    Some(SERVERLESS_ENDPOINT),
    Some("")
)]
fn gradient_provider_info_prefers_explicit_then_agent_then_serverless(
    #[case] explicit_base: Option<&str>,
    #[case] entries: &[(&str, &str)],
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let loaded = settings(entries);
    let key_override = explicit_key.map(secret);
    let (base, key) =
        AgentHost::Gradient.provider_info(&loaded, explicit_base, key_override.as_ref());
    assert_eq!(base.as_deref(), expected_base);
    assert_eq!(
        key.as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        expected_key
    );
}

#[test]
fn heroku_provider_info_has_no_default_base() {
    let loaded = settings(&[]);
    let (base, key) = AgentHost::Heroku.provider_info(&loaded, None, None);
    assert!(base.is_none());
    assert!(key.is_none());
    let with_environment = settings(&[
        ("HEROKU_API_BASE", "https://env"),
        ("HEROKU_API_KEY", "env"),
    ]);
    let (env_base, env_key) = AgentHost::Heroku.provider_info(&with_environment, None, None);
    assert_eq!(env_base.as_deref(), Some("https://env"));
    assert_eq!(
        env_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        Some("env")
    );
    let (explicit_base, explicit_key) = AgentHost::Heroku.provider_info(
        &with_environment,
        Some("https://explicit"),
        Some(&secret("explicit")),
    );
    assert_eq!(explicit_base.as_deref(), Some("https://explicit"));
    assert_eq!(
        explicit_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        Some("explicit")
    );
}

fn heroku_environment(base: Option<&str>) -> Vec<(&str, &str)> {
    base.map(|value| vec![("HEROKU_API_BASE", value)])
        .unwrap_or_default()
}

#[rstest]
#[case::appends_the_suffix(
    None,
    Some("https://heroku.example"),
    "https://heroku.example/v1/chat/completions"
)]
#[case::duplicates_the_version_of_a_v1_base(
    None,
    Some("https://heroku.example/v1"),
    "https://heroku.example/v1/v1/chat/completions"
)]
#[case::keeps_an_exact_suffix(
    None,
    Some("https://heroku.example/v1/chat/completions"),
    "https://heroku.example/v1/chat/completions"
)]
#[case::appends_after_a_trailing_suffix_slash(
    None,
    Some("https://heroku.example/v1/chat/completions/"),
    "https://heroku.example/v1/chat/completions//v1/chat/completions"
)]
#[case::explicit_base_wins(
    Some("https://explicit"),
    Some("https://heroku.example"),
    "https://explicit/v1/chat/completions"
)]
#[case::empty_explicit_base_uses_the_environment(
    Some(""),
    Some("https://heroku.example"),
    "https://heroku.example/v1/chat/completions"
)]
fn heroku_complete_url_requires_a_base_and_appends_the_suffix(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    let loaded = settings(&heroku_environment(environment_base));
    let url =
        HerokuChatConfig::get_complete_url(&loaded, api_base, None).expect("a base was found");
    assert_eq!(url, expected);
}

#[rstest]
#[case::missing_base(None, None)]
#[case::empty_environment_base(None, Some(""))]
#[case::empty_explicit_base(Some(""), None)]
#[case::both_bases_empty(Some(""), Some(""))]
fn heroku_complete_url_rejects_a_falsey_base(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
) {
    let loaded = settings(&heroku_environment(environment_base));
    let outcome = HerokuChatConfig::get_complete_url(&loaded, api_base, None);
    match outcome {
        Ok(_) => panic!("unexpected url"),
        Err(error) => {
            assert!(matches!(error, HerokuError::MissingBase));
            assert_eq!(error.status_code(), None);
        }
    }
}

#[test]
fn heroku_content_failures_keep_the_chat_transform_status() {
    let missing_file = HerokuError::Request(ChatContentError::MissingFile(MissingFileError {
        provider: "heroku",
        model: None,
    }));
    assert_eq!(missing_file.status_code(), Some(400));
    let source = serde_json::from_str::<Value>("{").expect_err("invalid json");
    let bad_shape = HerokuError::Request(ChatContentError::MessageShape { source });
    assert_eq!(bad_shape.status_code(), Some(500));
}

fn chat_messages(value: Value) -> Vec<ChatMessage> {
    serde_json::from_value(value).expect("chat messages")
}

#[test]
fn heroku_collapses_content_lists_before_the_openai_transform() {
    let messages = chat_messages(json!([
        {"role": "user", "content": [
            {"type": "text", "text": "hi"},
            {"type": "image_url", "image_url": "https://img.example"}
        ]},
        {"role": "user", "content": [{"type": "image_url", "image_url": "https://img.example"}]},
        {"role": "assistant", "content": "plain"}
    ]));
    let transformed =
        HerokuChatConfig::transform_request("m", &messages, &Object::new(), Mode::Sync)
            .expect("a plain request");
    let body = serde_json::to_value(transformed).expect("a JSON body");
    assert_eq!(body.get("model"), Some(&json!("m")));
    assert_eq!(body["messages"][0]["content"], json!("hi"));
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "image_url", "image_url": {"url": "https://img.example"}}])
    );
    assert_eq!(body["messages"][2]["content"], json!("plain"));
}

#[test]
fn heroku_keeps_the_openai_request_cleanup() {
    let messages = chat_messages(json!([
        {"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}
    ]));
    let optional = object(json!({"max_retries": 2_i32, "tool_choice": "auto"}));
    let transformed = HerokuChatConfig::transform_request("m", &messages, &optional, Mode::Sync)
        .expect("a plain request");
    let body = serde_json::to_value(transformed).expect("a JSON body");
    assert!(body.get("max_retries").is_none());
    assert!(body.get("tool_choice").is_none());
    assert!(body["messages"][0].get("cache_control").is_none());
}

#[test]
fn heroku_async_transform_gives_the_sync_body() {
    let messages = chat_messages(json!([
        {"role": "user", "content": [{"type": "text", "text": "hi"}]}
    ]));
    let optional = object(json!({"max_retries": 2_i32}));
    let sync = HerokuChatConfig::transform_request("m", &messages, &optional, Mode::Sync)
        .expect("a sync request");
    let asynchronous =
        HerokuChatConfig::transform_request("m", &messages, &optional, Mode::AsyncSubclass)
            .expect("an async request");
    assert_eq!(sync, asynchronous);
}

#[rstest]
#[case::hyperbolic(AgentHost::Hyperbolic, Some("hyperbolic"))]
#[case::lambda_ai(AgentHost::LambdaAi, Some("lambda_ai"))]
#[case::inception(AgentHost::Inception, Some("inception"))]
#[case::gradient(AgentHost::Gradient, None)]
#[case::heroku(AgentHost::Heroku, None)]
fn provider_tags_name_the_custom_provider(#[case] host: AgentHost, #[case] expected: Option<&str>) {
    assert_eq!(host.custom_llm_provider(), expected);
}

#[test]
fn only_hosts_with_an_override_name_a_supported_list() {
    assert_eq!(
        AgentHost::Hyperbolic.supported_params(),
        Some(HYPERBOLIC_PARAMS)
    );
    assert_eq!(
        AgentHost::Inception.supported_params(),
        Some(INCEPTION_PARAMS)
    );
    assert_eq!(AgentHost::LambdaAi.supported_params(), None);
    assert_eq!(AgentHost::Heroku.supported_params(), None);
}

#[test]
fn compatible_hosts_advertise_their_source_lists() {
    assert_eq!(
        HYPERBOLIC_PARAMS,
        [
            "messages",
            "model",
            "stream",
            "temperature",
            "top_p",
            "max_tokens",
            "frequency_penalty",
            "presence_penalty",
            "stop",
            "n",
            "tools",
            "tool_choice",
            "response_format",
            "seed",
            "user",
        ]
    );
    assert_eq!(
        INCEPTION_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "stop",
            "tools",
            "tool_choice",
            "stream",
            "stream_options",
            "response_format",
            "reasoning_effort",
            "reasoning_summary",
            "reasoning_summary_wait",
            "diffusing",
            "realtime",
        ]
    );
}

#[rstest]
#[case::hyperbolic_default(
    ("HYPERBOLIC_API_BASE", "HYPERBOLIC_API_KEY"),
    None,
    None,
    Some("https://api.hyperbolic.xyz/v1"),
    None
)]
#[case::hyperbolic_environment(
    ("HYPERBOLIC_API_BASE", "HYPERBOLIC_API_KEY"),
    Some("https://hyperbolic.example/v1"),
    Some("hyperbolic"),
    Some("https://hyperbolic.example/v1"),
    Some("hyperbolic")
)]
#[case::lambda_default(
    ("LAMBDA_API_BASE", "LAMBDA_API_KEY"),
    None,
    None,
    Some("https://api.lambda.ai/v1"),
    None
)]
#[case::lambda_environment(
    ("LAMBDA_API_BASE", "LAMBDA_API_KEY"),
    Some("https://lambda.example/v1"),
    Some("lambda"),
    Some("https://lambda.example/v1"),
    Some("lambda")
)]
fn compatible_hosts_read_their_own_environment(
    #[case] names: (&str, &str),
    #[case] environment_base: Option<&str>,
    #[case] environment_key: Option<&str>,
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let (base_name, key_name) = names;
    let mut entries: Vec<(&str, &str)> = Vec::new();
    if let Some(base) = environment_base {
        entries.push((base_name, base));
    }
    if let Some(key) = environment_key {
        entries.push((key_name, key));
    }
    let loaded = settings(&entries);
    let host = match base_name {
        "HYPERBOLIC_API_BASE" => AgentHost::Hyperbolic,
        _ => AgentHost::LambdaAi,
    };
    let (base, key) = host.provider_info(&loaded, None, None);
    assert_eq!(base.as_deref(), expected_base);
    assert_eq!(
        key.as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        expected_key
    );
}

fn inception_settings(
    environment_base: Option<&str>,
    environment_key: Option<&str>,
    global_key: Option<&str>,
) -> Settings {
    let mut entries: Vec<(&str, &str)> = Vec::new();
    if let Some(base) = environment_base {
        entries.push(("INCEPTION_API_BASE", base));
    }
    if let Some(key) = environment_key {
        entries.push(("INCEPTION_API_KEY", key));
    }
    let mut loaded = settings(&entries);
    loaded.inception_key = global_key.map(secret);
    loaded
}

#[rstest]
#[case::default_base((None, None), None, (None, None), (Some(INCEPTION_API_BASE), None))]
#[case::environment_key(
    (None, None),
    None,
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), Some("env"))
)]
#[case::global_key_wins(
    (None, None),
    Some("global"),
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), Some("global"))
)]
#[case::empty_global_key_falls_to_environment(
    (None, None),
    Some(""),
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), Some("env"))
)]
#[case::explicit_key(
    (None, Some("explicit")),
    Some("global"),
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), Some("explicit"))
)]
#[case::environment_base_alone(
    (None, None),
    None,
    (Some("https://env"), Some("env")),
    (Some("https://env"), Some("env"))
)]
#[case::explicit_base_without_key_skips_the_fallbacks(
    (Some("https://custom"), None),
    Some("global"),
    (Some("https://env"), Some("env")),
    (Some("https://custom"), None)
)]
#[case::explicit_base_with_key_keeps_it(
    (Some("https://custom"), Some("explicit")),
    None,
    (Some("https://env"), Some("env")),
    (Some("https://custom"), Some("explicit"))
)]
#[case::explicit_empty_base_alone(
    (Some(""), None),
    Some("global"),
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), None)
)]
#[case::explicit_empty_base_uses_the_environment_base(
    (Some(""), None),
    None,
    (Some("https://env"), Some("env")),
    (Some("https://env"), None)
)]
#[case::explicit_empty_base_with_empty_key(
    (Some(""), Some("")),
    Some("global"),
    (None, Some("env")),
    (Some(INCEPTION_API_BASE), Some(""))
)]
fn inception_borrows_ambient_keys_only_while_no_base_was_passed(
    #[case] explicit: (Option<&str>, Option<&str>),
    #[case] global_key: Option<&str>,
    #[case] environment: (Option<&str>, Option<&str>),
    #[case] expected: (Option<&str>, Option<&str>),
) {
    let loaded = inception_settings(environment.0, environment.1, global_key);
    let key_override = explicit.1.map(secret);
    let (base, key) =
        AgentHost::Inception.provider_info(&loaded, explicit.0, key_override.as_ref());
    assert_eq!(base.as_deref(), expected.0);
    assert_eq!(
        key.as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        expected.1
    );
}

#[test]
fn inception_reads_the_global_key_from_the_config() {
    let loaded = settings_from("litellm_settings:\n  inception_key: global-key\n");
    assert_eq!(
        loaded
            .inception_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("global-key")
    );
    let (base, key) = AgentHost::Inception.provider_info(&loaded, None, None);
    assert_eq!(base.as_deref(), Some(INCEPTION_API_BASE));
    assert_eq!(
        key.as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        Some("global-key")
    );
}

#[test]
fn inception_text_completion_lists_the_fim_options() {
    assert_eq!(
        inception_fim::SUPPORTED_PARAMS,
        [
            "suffix",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "frequency_penalty",
            "presence_penalty",
            "stop",
            "stream",
            "stream_options",
        ]
    );
}

#[rstest]
#[case::only_max_completion_tokens(vec![("max_completion_tokens", json!(5_i32))], json!({"max_tokens": 5_i32}))]
#[case::only_max_tokens(vec![("max_tokens", json!(7_i32))], json!({"max_tokens": 7_i32}))]
#[case::declared_limit_last(
    vec![("max_completion_tokens", json!(5_i32)), ("max_tokens", json!(7_i32))],
    json!({"max_tokens": 7_i32})
)]
#[case::completion_limit_last(
    vec![("max_tokens", json!(7_i32)), ("max_completion_tokens", json!(5_i32))],
    json!({"max_tokens": 5_i32})
)]
#[case::nulls_and_zero(
    vec![("suffix", Value::Null), ("top_p", json!(0_i32))],
    json!({"suffix": null, "top_p": 0_i32})
)]
#[case::unsupported_is_skipped(
    vec![("unsupported", json!(1_i32)), ("stream", json!(true))],
    json!({"stream": true})
)]
fn inception_fim_maps_limits_in_input_order(
    #[case] requested: Vec<(&str, Value)>,
    #[case] expected: Value,
) {
    let mapped: IndexMap<String, Value> = requested
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    let mut existing = Object::new();
    InceptionTextCompletionConfig::map_params(&mapped, &mut existing);
    assert_eq!(Value::Object(existing), expected);
}

#[test]
fn inception_fim_mapping_keeps_existing_options_and_overrides() {
    let requested = params(json!({"max_completion_tokens": 5_i32}));
    let mut existing = object(json!({"max_tokens": 1_i32, "stream": false}));
    InceptionTextCompletionConfig::map_params(&requested, &mut existing);
    assert_eq!(
        Value::Object(existing),
        json!({"max_tokens": 5_i32, "stream": false})
    );
}
