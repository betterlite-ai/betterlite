#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::{
    DEFAULT_API_BASE, TencentParams, complete_url, provider_info, supported_params,
    thinking_payload,
};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    Value::Object(object(text))
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn map(params: &str, options: &str, supports_reasoning: bool, supports_adaptive: bool) -> Object {
    let mut mapped = object(options);
    TencentParams {
        config: &config(),
        catalog: &catalog(),
        supports_reasoning,
        supports_adaptive_thinking: supports_adaptive,
    }
    .map("hunyuan-turbos", &object(params), &mut mapped);
    mapped
}

#[test]
fn a_reasoning_model_gains_thinking_then_reasoning_effort_after_the_base_list() {
    let params = supported_params("hunyuan-turbos", &config(), &catalog(), true);
    let mut expected = config().supported_params("hunyuan-turbos", &catalog());
    expected.extend(["thinking", "reasoning_effort"]);
    assert_eq!(params, expected);
}

#[test]
fn a_model_that_does_not_reason_keeps_only_the_base_list() {
    assert_eq!(
        supported_params("hunyuan-turbos", &config(), &catalog(), false),
        config().supported_params("hunyuan-turbos", &catalog())
    );
}

#[test]
fn the_default_base_is_the_tokenhub_international_endpoint() {
    assert_eq!(
        DEFAULT_API_BASE,
        "https://tokenhub-intl.tencentcloudmaas.com/v1"
    );
}

#[rstest]
#[case::default_base(None, "https://tokenhub-intl.tencentcloudmaas.com/v1/chat/completions")]
#[case::v1_base(Some("https://host.test/v1"), "https://host.test/v1/chat/completions")]
#[case::bare_host(Some("https://host.test"), "https://host.test/v1/chat/completions")]
#[case::trailing_slash(Some("https://host.test/"), "https://host.test/v1/chat/completions")]
#[case::many_trailing_slashes(
    Some("https://host.test/v1///"),
    "https://host.test/v1/chat/completions"
)]
#[case::complete_url(
    Some("https://host.test/v1/chat/completions"),
    "https://host.test/v1/chat/completions"
)]
#[case::complete_url_with_slash(
    Some("https://host.test/v1/chat/completions/"),
    "https://host.test/v1/chat/completions"
)]
#[case::empty_is_the_default(
    Some(""),
    "https://tokenhub-intl.tencentcloudmaas.com/v1/chat/completions"
)]
fn the_base_is_trimmed_and_gains_v1_then_the_chat_path(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn an_explicit_base_and_key_win_over_the_environment() {
    let explicit_key = key("explicit-key");
    let environment_key = key("env-key");
    let (base, chosen) = provider_info(
        Some("https://explicit.test/v1"),
        Some("https://env.test/v1"),
        Some(&explicit_key),
        Some(&environment_key),
    );
    assert_eq!(base, "https://explicit.test/v1");
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("explicit-key".to_owned())
    );
}

#[test]
fn the_environment_values_are_used_when_the_call_has_none() {
    let environment_key = key("env-key");
    let (base, chosen) = provider_info(
        None,
        Some("https://env.test/v1"),
        None,
        Some(&environment_key),
    );
    assert_eq!(base, "https://env.test/v1");
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("env-key".to_owned())
    );
}

#[test]
fn an_empty_value_counts_as_unset_at_every_step() {
    let explicit_key = key("");
    let environment_key = key("");
    let (base, chosen) = provider_info(
        Some(""),
        Some(""),
        Some(&explicit_key),
        Some(&environment_key),
    );
    assert_eq!(base, DEFAULT_API_BASE);
    assert!(chosen.is_none());
}

#[rstest]
#[case::none_effort(r#"{"type": "enabled"}"#, Some("none"), r#"{"type": "disabled"}"#)]
#[case::high_effort(r#"{"type": "enabled"}"#, Some("high"), r#"{"type": "enabled"}"#)]
#[case::empty_effort_is_enabled(r#"{"type": "enabled"}"#, Some(""), r#"{"type": "enabled"}"#)]
#[case::thinking_object_wins(
    r#"{"type": "disabled", "budget_tokens": 3}"#,
    Some("high"),
    r#"{"type": "disabled", "budget_tokens": 3}"#
)]
#[case::thinking_object_keeps_other_keys(
    r#"{"type": "enabled", "display": "summarized"}"#,
    None,
    r#"{"type": "enabled", "display": "summarized"}"#
)]
fn a_thinking_object_is_the_payload_and_otherwise_the_effort_decides(
    #[case] thinking: &str,
    #[case] effort: Option<&str>,
    #[case] expected: &str,
) {
    let effort_value = effort.map(|text| Value::String(text.to_owned()));
    assert_eq!(
        thinking_payload(Some(&value(thinking)), effort_value.as_ref(), false),
        Some(object(expected))
    );
}

#[rstest]
#[case::non_string_effort(3)]
#[case::null_effort(Value::Null)]
#[case::bool_effort(true)]
fn a_non_string_effort_gives_no_payload(#[case] effort: Value) {
    assert_eq!(thinking_payload(None, Some(&effort), false), None);
}

#[test]
fn no_thinking_and_no_effort_gives_no_payload() {
    assert_eq!(thinking_payload(None, None, true), None);
}

#[rstest]
#[case::enabled_with_integer_budget(
    r#"{"type": "enabled", "budget_tokens": 7}"#,
    r#"{"type": "adaptive", "budget_tokens": 7}"#
)]
#[case::enabled_without_budget(r#"{"type": "enabled"}"#, r#"{"type": "adaptive"}"#)]
#[case::enabled_with_float_budget(
    r#"{"type": "enabled", "budget_tokens": 7.5}"#,
    r#"{"type": "adaptive"}"#
)]
#[case::enabled_with_text_budget(
    r#"{"type": "enabled", "budget_tokens": "7"}"#,
    r#"{"type": "adaptive"}"#
)]
#[case::enabled_with_null_budget(
    r#"{"type": "enabled", "budget_tokens": null}"#,
    r#"{"type": "adaptive"}"#
)]
#[case::enabled_with_bool_budget(
    r#"{"type": "enabled", "budget_tokens": true}"#,
    r#"{"type": "adaptive"}"#
)]
fn an_adaptive_model_turns_enabled_into_adaptive_and_keeps_only_an_integer_budget(
    #[case] thinking: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        thinking_payload(Some(&value(thinking)), None, true),
        Some(object(expected))
    );
}

#[test]
fn an_enabled_payload_on_a_non_adaptive_model_is_unchanged() {
    assert_eq!(
        thinking_payload(
            Some(&value(r#"{"type": "enabled", "budget_tokens": 7.5}"#)),
            None,
            false
        ),
        Some(object(r#"{"type": "enabled", "budget_tokens": 7.5}"#))
    );
}

#[rstest]
#[case::disabled(r#"{"type": "disabled", "budget_tokens": 3}"#)]
#[case::other_type(r#"{"type": "other"}"#)]
#[case::missing_type(r#"{"budget_tokens": 3}"#)]
fn a_payload_that_is_not_enabled_passes_unchanged_on_an_adaptive_model(#[case] thinking: &str) {
    assert_eq!(
        thinking_payload(Some(&value(thinking)), None, true),
        Some(object(thinking))
    );
}

#[test]
fn a_thinking_value_that_is_not_an_object_falls_through_to_the_effort() {
    assert_eq!(
        thinking_payload(
            Some(&Value::Bool(true)),
            Some(&Value::String(String::from("high"))),
            false
        ),
        Some(object(r#"{"type": "enabled"}"#))
    );
}

#[test]
fn neither_reasoning_name_survives_at_the_top_level() {
    let mapped = map(
        r#"{"thinking": {"type": "enabled"}, "reasoning_effort": "high", "temperature": 0.2}"#,
        "{}",
        true,
        false,
    );
    assert_eq!(
        mapped,
        object(r#"{"temperature": 0.2, "extra_body": {"thinking": {"type": "enabled"}}}"#)
    );
}

#[test]
fn the_effort_string_becomes_the_thinking_type_when_no_object_is_given() {
    let mapped = map(r#"{"reasoning_effort": "none"}"#, "{}", true, false);
    assert_eq!(
        mapped,
        object(r#"{"extra_body": {"thinking": {"type": "disabled"}}}"#)
    );
}

#[test]
fn an_unsupported_reasoning_name_is_not_copied_from_the_request() {
    let mapped = map(r#"{"thinking": {"type": "enabled"}}"#, "{}", false, false);
    assert_eq!(mapped, object("{}"));
}

#[test]
fn a_reasoning_name_already_in_the_options_is_still_moved_into_extra_body() {
    let mapped = map(
        "{}",
        r#"{"thinking": {"type": "disabled"}, "temperature": 0}"#,
        true,
        false,
    );
    assert_eq!(
        mapped,
        object(r#"{"temperature": 0, "extra_body": {"thinking": {"type": "disabled"}}}"#)
    );
}

#[test]
fn extra_body_is_replaced_by_the_thinking_payload() {
    let mapped = map(
        r#"{"thinking": {"type": "enabled"}}"#,
        r#"{"extra_body": {"keep": 1, "thinking": {"type": "disabled"}}}"#,
        true,
        false,
    );
    assert_eq!(
        mapped,
        object(r#"{"extra_body": {"thinking": {"type": "enabled"}}}"#)
    );
}

#[test]
fn an_enabled_request_on_an_adaptive_model_is_coerced_through_the_mapper() {
    let mapped = map(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 11}}"#,
        "{}",
        true,
        true,
    );
    assert_eq!(
        mapped,
        object(r#"{"extra_body": {"thinking": {"type": "adaptive", "budget_tokens": 11}}}"#)
    );
}

#[test]
fn other_supported_options_are_copied_as_given() {
    let mapped = map(
        r#"{"temperature": 0, "stream": false, "seed": 4, "stop": "", "tools": [], "tool_choice": "auto", "response_format": {"type": "json_object"}, "frequency_penalty": 0.1}"#,
        "{}",
        false,
        false,
    );
    assert_eq!(
        mapped,
        object(
            r#"{"temperature": 0, "stream": false, "seed": 4, "stop": "", "tools": [], "tool_choice": "auto", "response_format": {"type": "json_object"}, "frequency_penalty": 0.1}"#
        )
    );
}
