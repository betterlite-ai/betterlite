#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Sections;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::CompatibleChatAlias;
use super::baseten::{self, BasetenSettings};
use super::cerebras::{self, CerebrasSettings};
use super::featherless::{
    self, ChatIdentity, DropParams, FeatherlessSettings, HostedChatOptionsError,
};
use super::map_renaming_token_limit;
use crate::base_helpers::provider_config;
use crate::openai_base_config::Headers;

fn loaded(entries: &[(&str, &str)]) -> Sections {
    let lookup: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("the config loads")
        .sections
}

fn params(text: &str) -> IndexMap<String, Value> {
    serde_json::from_str(text).expect("an ordered parameter map")
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("a JSON value")
}

fn snapshot<Config>(config: &Config) -> Value
where
    Config: serde::Serialize,
{
    Value::Object(provider_config(config).expect("the snapshot is an object"))
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn keys(existing: &IndexMap<String, Value>) -> Vec<&str> {
    existing.keys().map(String::as_str).collect()
}

#[test]
fn baseten_advertises_the_pinned_fourteen_options() {
    assert_eq!(
        baseten::SUPPORTED_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "response_format",
            "seed",
            "stop",
            "stream",
            "temperature",
            "top_p",
            "tool_choice",
            "tools",
            "user",
            "presence_penalty",
            "frequency_penalty",
            "stream_options",
        ]
    );
}

#[test]
fn cerebras_advertises_its_pinned_options_and_follows_the_capability() {
    assert_eq!(
        cerebras::SUPPORTED_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "response_format",
            "seed",
            "stop",
            "stream",
            "temperature",
            "top_p",
            "tool_choice",
            "tools",
            "user",
            "max_retries",
            "extra_headers",
        ]
    );
    assert_eq!(
        cerebras::supported_params(false),
        cerebras::SUPPORTED_PARAMS.to_vec()
    );
    let mut with_reasoning = cerebras::SUPPORTED_PARAMS.to_vec();
    with_reasoning.push("reasoning_effort");
    assert_eq!(cerebras::supported_params(true), with_reasoning);
}

#[test]
fn featherless_advertises_the_pinned_twelve_options() {
    assert_eq!(
        featherless::SUPPORTED_PARAMS,
        [
            "stream",
            "frequency_penalty",
            "function_call",
            "functions",
            "logit_bias",
            "max_tokens",
            "max_completion_tokens",
            "n",
            "presence_penalty",
            "stop",
            "temperature",
            "top_p",
        ]
    );
}

#[rstest]
#[case::limit_after_completion(
    r#"{"max_tokens": 9, "max_completion_tokens": 5}"#,
    r#"{"temperature": 0.5, "max_tokens": 5}"#
)]
#[case::completion_after_limit(
    r#"{"max_completion_tokens": 5, "max_tokens": 9}"#,
    r#"{"temperature": 0.5, "max_tokens": 9}"#
)]
#[case::null_completion(
    r#"{"max_completion_tokens": null}"#,
    r#"{"temperature": 0.5, "max_tokens": null}"#
)]
fn baseten_maps_the_token_limit_in_input_order(#[case] requested: &str, #[case] expected: &str) {
    let mut existing = params(r#"{"temperature": 0.5}"#);
    map_renaming_token_limit(&params(requested), &mut existing, baseten::SUPPORTED_PARAMS);
    assert_eq!(
        Value::Object(existing.into_iter().collect()),
        value(expected)
    );
}

#[test]
fn baseten_appends_new_options_after_the_existing_ones() {
    let mut existing = params(r#"{"temperature": 0.5}"#);
    map_renaming_token_limit(
        &params(r#"{"seed": 1, "stop": ["a"]}"#),
        &mut existing,
        baseten::SUPPORTED_PARAMS,
    );
    assert_eq!(keys(&existing), ["temperature", "seed", "stop"]);
}

#[test]
fn baseten_copies_supported_nulls_and_omits_unknown_fields() {
    let mut existing = IndexMap::new();
    map_renaming_token_limit(
        &params(r#"{"response_format": null, "seed": 7, "unknown": 1, "stream": true}"#),
        &mut existing,
        baseten::SUPPORTED_PARAMS,
    );
    assert_eq!(
        Value::Object(existing.into_iter().collect()),
        value(r#"{"response_format": null, "seed": 7, "stream": true}"#)
    );
}

#[test]
fn cerebras_copies_only_its_supported_subset() {
    let requested = params(
        r#"{"presence_penalty": 1, "frequency_penalty": 1, "stream_options": {"include_usage": true}, "max_retries": 2, "extra_headers": {"x-a": "b"}, "reasoning_effort": "high"}"#,
    );
    let mut without_reasoning = IndexMap::new();
    map_renaming_token_limit(
        &requested,
        &mut without_reasoning,
        &cerebras::supported_params(false),
    );
    assert_eq!(
        Value::Object(without_reasoning.into_iter().collect()),
        value(r#"{"max_retries": 2, "extra_headers": {"x-a": "b"}}"#)
    );
    let mut with_reasoning = IndexMap::new();
    map_renaming_token_limit(
        &requested,
        &mut with_reasoning,
        &cerebras::supported_params(true),
    );
    assert_eq!(
        Value::Object(with_reasoning.into_iter().collect()),
        value(r#"{"max_retries": 2, "extra_headers": {"x-a": "b"}, "reasoning_effort": "high"}"#)
    );
}

#[test]
fn cerebras_renames_a_null_token_limit() {
    let mut existing = IndexMap::new();
    map_renaming_token_limit(
        &params(r#"{"max_completion_tokens": null}"#),
        &mut existing,
        &cerebras::supported_params(false),
    );
    assert_eq!(
        Value::Object(existing.into_iter().collect()),
        value(r#"{"max_tokens": null}"#)
    );
}

#[test]
fn baseten_snapshot_starts_empty_and_keeps_supplied_falsey_values() {
    assert_eq!(snapshot(&BasetenSettings::default()), value("{}"));
    let given: BasetenSettings = serde_json::from_str(
        r#"{
        "max_tokens": 0,
        "stream": false,
        "stop": [],
        "temperature": 0.5,
        "tools": null
    }"#,
    )
    .expect("a settings snapshot");
    assert_eq!(
        snapshot(&given),
        value(r#"{"max_tokens": 0, "stream": false, "stop": [], "temperature": 0.5}"#)
    );
}

#[test]
fn cerebras_snapshot_keeps_reasoning_effort_and_temperature() {
    let given: CerebrasSettings =
        serde_json::from_str(r#"{"reasoning_effort": "low", "temperature": 0.0}"#)
            .expect("a settings snapshot");
    assert_eq!(
        snapshot(&given),
        value(r#"{"reasoning_effort": "low", "temperature": 0.0}"#)
    );
}

#[test]
fn snapshots_reject_fields_their_constructor_does_not_take() {
    serde_json::from_str::<CerebrasSettings>(r#"{"max_retries": 2}"#)
        .expect_err("an unknown field is rejected");
    serde_json::from_str::<BasetenSettings>(r#"{"reasoning_effort": "low"}"#)
        .expect_err("an unknown field is rejected");
    serde_json::from_str::<FeatherlessSettings>(r#"{"stream": true}"#)
        .expect_err("an unknown field is rejected");
}

#[test]
fn featherless_snapshot_renames_n_and_keeps_falsey_values() {
    let given: FeatherlessSettings = serde_json::from_str(
        r#"{
        "n": 3,
        "temperature": 0,
        "logit_bias": {},
        "functions": []
    }"#,
    )
    .expect("a settings snapshot");
    assert_eq!(
        snapshot(&given),
        value(r#"{"n": 3, "temperature": 0, "logit_bias": {}, "functions": []}"#)
    );
}

#[test]
fn each_config_keeps_its_own_snapshot() {
    let first: BasetenSettings = serde_json::from_str(r#"{"max_tokens": 5}"#).expect("a snapshot");
    let second: BasetenSettings =
        serde_json::from_str(r#"{"temperature": 0.5}"#).expect("a snapshot");
    assert_eq!(snapshot(&first), value(r#"{"max_tokens": 5}"#));
    assert_eq!(snapshot(&second), value(r#"{"temperature": 0.5}"#));
}

#[rstest]
#[case::tools_list(r#"{"tools": [{"type": "function"}]}"#, "tools")]
#[case::tools_empty(r#"{"tools": []}"#, "tools")]
#[case::tools_null(r#"{"tools": null}"#, "tools")]
#[case::choice_required(r#"{"tool_choice": "required"}"#, "tool_choice")]
#[case::choice_null(r#"{"tool_choice": null}"#, "tool_choice")]
#[case::choice_object(r#"{"tool_choice": {"type": "function"}}"#, "tool_choice")]
fn featherless_rejects_tool_options_unless_dropping(#[case] requested: &str, #[case] name: &str) {
    let error = featherless::map_openai_params(
        &params(requested),
        &mut IndexMap::new(),
        DropParams {
            argument: false,
            global: false,
        },
    )
    .expect_err("the tool option is rejected");
    assert_eq!(
        error,
        HostedChatOptionsError::UnsupportedParam {
            name: name.to_owned()
        }
    );
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::argument(DropParams { argument: true, global: false })]
#[case::global(DropParams { argument: false, global: true })]
fn featherless_drops_tool_options_when_either_flag_is_true(#[case] drop: DropParams) {
    let mut existing = params(r#"{"temperature": 0.5}"#);
    featherless::map_openai_params(
        &params(r#"{"tools": [{"type": "function"}], "tool_choice": "required"}"#),
        &mut existing,
        drop,
    )
    .expect("dropping skips both");
    assert_eq!(keys(&existing), ["temperature"]);
}

#[test]
fn featherless_forwards_legacy_function_options() {
    let mut existing = IndexMap::new();
    featherless::map_openai_params(
        &params(r#"{"function_call": "auto", "functions": [{"name": "f"}], "function_call_unknown": null}"#),
        &mut existing,
        DropParams {
            argument: false,
            global: false,
        },
    )
    .expect("legacy function options are forwarded");
    assert_eq!(
        Value::Object(existing.into_iter().collect()),
        value(r#"{"function_call": "auto", "functions": [{"name": "f"}]}"#)
    );
}

#[rstest]
#[case::choice_auto(r#"{"tool_choice": "auto"}"#, r#"{"tool_choice": "auto"}"#)]
#[case::choice_none(r#"{"tool_choice": "none"}"#, r#"{"tool_choice": "none"}"#)]
#[case::renamed_null(r#"{"max_completion_tokens": null}"#, r#"{"max_tokens": null}"#)]
#[case::nulls_omitted(
    r#"{"temperature": null, "stop": null, "max_tokens": null, "n": null}"#,
    "{}"
)]
#[case::later_limit_wins(
    r#"{"max_completion_tokens": 3, "max_tokens": null}"#,
    r#"{"max_tokens": null}"#
)]
fn featherless_maps_the_accepted_options(#[case] requested: &str, #[case] expected: &str) {
    let mut existing = IndexMap::new();
    featherless::map_openai_params(
        &params(requested),
        &mut existing,
        DropParams {
            argument: false,
            global: false,
        },
    )
    .expect("the options are mapped");
    assert_eq!(
        Value::Object(existing.into_iter().collect()),
        value(expected)
    );
}

#[test]
fn featherless_validation_requires_a_truthy_key() {
    let missing =
        featherless::validate_environment(None, None).expect_err("a missing key is rejected");
    assert_eq!(missing, HostedChatOptionsError::MissingApiKey);
    assert_eq!(missing.status_code(), None);
    let empty = featherless::validate_environment(Some(&secret("")), None)
        .expect_err("an empty key is rejected");
    assert_eq!(empty, HostedChatOptionsError::MissingApiKey);
}

#[test]
fn featherless_validation_overwrites_the_two_headers_it_owns() {
    let given: Headers = serde_json::from_str(
        r#"{
        "Authorization": "mine",
        "content-type": "text/plain",
        "x-extra": "1"
    }"#,
    )
    .expect("a header map");
    let validated = featherless::validate_environment(Some(&secret("sk-featherless")), Some(given))
        .expect("a key is present");
    assert_eq!(
        Value::Object(
            validated
                .into_iter()
                .map(|(name, value)| (name, Value::String(value)))
                .collect()
        ),
        value(
            r#"{"Authorization": "Bearer sk-featherless", "Content-Type": "application/json", "content-type": "text/plain", "x-extra": "1"}"#
        )
    );
}

#[test]
fn featherless_validation_creates_headers_when_none_are_given() {
    let validated = featherless::validate_environment(Some(&secret("sk-featherless")), None)
        .expect("a key is present");
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-featherless")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.len(), 2);
}

#[rstest]
#[case::featherless_defaults(
    ChatIdentity::Featherless,
    &[],
    None,
    None,
    (None, "https://api.featherless.ai/v1")
)]
#[case::featherless_primary_environment(
    ChatIdentity::Featherless,
    &[
        ("FEATHERLESS_AI_API_BASE", "https://primary.example"),
        ("FEATHERLESS_AI_API_KEY", "sk-primary"),
    ],
    None,
    None,
    (Some("sk-primary"), "https://primary.example")
)]
#[case::featherless_secondary_environment(
    ChatIdentity::Featherless,
    &[
        ("FEATHERLESS_API_BASE", "https://secondary.example"),
        ("FEATHERLESS_API_KEY", "sk-secondary"),
    ],
    None,
    None,
    (Some("sk-secondary"), "https://secondary.example")
)]
#[case::featherless_primary_environment_wins(
    ChatIdentity::Featherless,
    &[
        ("FEATHERLESS_AI_API_BASE", "https://primary.example"),
        ("FEATHERLESS_AI_API_KEY", "sk-primary"),
        ("FEATHERLESS_API_BASE", "https://secondary.example"),
        ("FEATHERLESS_API_KEY", "sk-secondary"),
    ],
    None,
    None,
    (Some("sk-primary"), "https://primary.example")
)]
#[case::featherless_explicit_wins(
    ChatIdentity::Featherless,
    &[
        ("FEATHERLESS_AI_API_BASE", "https://primary.example"),
        ("FEATHERLESS_AI_API_KEY", "sk-primary"),
    ],
    Some("https://explicit.example"),
    Some("sk-explicit"),
    (Some("sk-explicit"), "https://explicit.example")
)]
#[case::featherless_empty_explicit_falls_through(
    ChatIdentity::Featherless,
    &[
        ("FEATHERLESS_AI_API_BASE", "https://primary.example"),
        ("FEATHERLESS_AI_API_KEY", "sk-primary"),
    ],
    Some(""),
    Some(""),
    (Some("sk-primary"), "https://primary.example")
)]
#[case::aiml_defaults(
    ChatIdentity::Aiml,
    &[],
    None,
    None,
    (None, "https://api.aimlapi.com/v1")
)]
#[case::aiml_environment(
    ChatIdentity::Aiml,
    &[
        ("AIML_API_BASE", "https://aiml.example/v1"),
        ("AIML_API_KEY", "sk-aiml"),
    ],
    None,
    None,
    (Some("sk-aiml"), "https://aiml.example/v1")
)]
#[case::aiml_explicit(
    ChatIdentity::Aiml,
    &[
        ("AIML_API_BASE", "https://aiml.example/v1"),
        ("AIML_API_KEY", "sk-aiml"),
    ],
    Some("https://explicit.example/v1"),
    Some("sk-explicit"),
    (Some("sk-explicit"), "https://explicit.example/v1")
)]
#[case::aiml_image_alias_is_not_read(
    ChatIdentity::Aiml,
    &[("AIMLAPI_KEY", "sk-image-only")],
    None,
    None,
    (None, "https://api.aimlapi.com/v1")
)]
fn chat_identity_resolves_its_chain(
    #[case] host: ChatIdentity,
    #[case] entries: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected: (Option<&str>, &str),
) {
    let (expected_key, expected_base) = expected;
    let settings = loaded(entries).hosted_chat_settings;
    let key_override = explicit_key.map(secret);
    let (base, key) =
        featherless::chat_identity(host, &settings, explicit_base, key_override.as_ref());
    assert_eq!(base, expected_base);
    assert_eq!(
        key.as_ref().map(|secret| secret.expose_secret().to_owned()),
        expected_key.map(str::to_owned)
    );
}

#[rstest]
#[case::empower(CompatibleChatAlias::Empower, "empower")]
#[case::friendliai(CompatibleChatAlias::Friendliai, "friendliai")]
#[case::galadriel(CompatibleChatAlias::Galadriel, "galadriel")]
#[case::github(CompatibleChatAlias::Github, "github")]
fn compatible_aliases_keep_their_provider_tags(
    #[case] alias: CompatibleChatAlias,
    #[case] provider: &str,
) {
    assert_eq!(alias.provider(), provider);
    let parsed: CompatibleChatAlias =
        serde_json::from_value(Value::String(provider.to_owned())).expect("a provider name");
    assert_eq!(parsed, alias);
}

#[rstest]
#[case::empower(CompatibleChatAlias::Empower, "empower")]
#[case::friendliai(CompatibleChatAlias::Friendliai, "friendliai")]
#[case::galadriel(CompatibleChatAlias::Galadriel, "galadriel")]
#[case::github(CompatibleChatAlias::Github, "github")]
fn compatible_aliases_share_the_b1425_identity(
    #[case] alias: CompatibleChatAlias,
    #[case] provider: &str,
) {
    let settings = loaded(&[
        ("OPENAI_LIKE_API_BASE", "https://like.example/v1"),
        ("OPENAI_LIKE_API_KEY", "sk-like"),
    ])
    .openai_like_settings;
    let (base, key) = settings.resolve(None, None);
    assert_eq!(alias.provider(), provider);
    assert_eq!(base.as_deref(), Some("https://like.example/v1"));
    assert_eq!(key.expose_secret(), "sk-like");
}
