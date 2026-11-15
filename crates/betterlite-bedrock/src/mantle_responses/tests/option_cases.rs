use crate::mantle_responses::error::MantleResponsesError;

use super::support;

const MODEL: &str = "openai.gpt-oss-120b";

pub(super) fn mantle_response_options() {
    mapped_table_cases();
    unhashable_tier_cases();
    case_sensitive_cases();
    summary_cases();
    standard_path_cases();
}

fn case_sensitive_cases() {
    let (_messages, _guard) = support::capture();
    let error = support::rejected(MODEL, r#"{"service_tier": "AUTO"}"#, false);
    assert!(matches!(
        &error,
        MantleResponsesError::UnsupportedParams { .. }
    ));
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        support::mapped(MODEL, r#"{"service_tier": "AUTO"}"#, true),
        support::object("{}")
    );
}

fn unhashable_tier_cases() {
    let (_messages, _guard) = support::capture();
    let listed = support::rejected(MODEL, r#"{"service_tier": ["auto"]}"#, false);
    assert!(matches!(
        &listed,
        MantleResponsesError::InvalidParams { .. }
    ));
    assert_eq!(listed.status_code(), Some(400));
    let mapped = support::rejected(MODEL, r#"{"service_tier": {"tier": "auto"}}"#, true);
    assert!(matches!(
        &mapped,
        MantleResponsesError::InvalidParams { .. }
    ));
}

fn summary_cases() {
    assert_eq!(
        support::mapped(MODEL, r#"{"reasoning": {"summary": "auto"}}"#, false),
        support::object(r#"{"reasoning": {"summary": "auto"}}"#)
    );
    assert_eq!(
        support::mapped(MODEL, r#"{"reasoning": {"summary": null}}"#, false),
        support::object(r#"{"reasoning": {"summary": null}}"#)
    );
    assert_eq!(
        support::mapped(MODEL, r#"{"reasoning": {"effort": "high"}}"#, false),
        support::object(r#"{"reasoning": {"effort": "high"}}"#)
    );
    let (_messages, _guard) = support::capture();
    let error = support::rejected(MODEL, r#"{"reasoning": {"summary": "concise"}}"#, false);
    let MantleResponsesError::UnsupportedParams { message } = &error else {
        panic!("expected an unsupported params error");
    };
    assert_eq!(
        message,
        "bedrock_mantle does not support reasoning.summary='concise' for 'openai.gpt-oss-120b'; the Bedrock Mantle OpenAI Responses path only accepts 'auto'. Set `drop_params: true` (litellm_settings or this deployment's litellm_params) to have LiteLLM drop it, or set `model_reasoning_summary = \"auto\"` in the client (Codex CLI: ~/.codex/config.toml)."
    );
}

fn standard_path_cases() {
    let settings = support::config(false);
    assert_eq!(
        support::map_with(
            &settings,
            MODEL,
            r#"{"reasoning": {"summary": "detailed"}}"#,
            false
        )
        .expect("the standard path keeps summaries"),
        support::object(r#"{"reasoning": {"summary": "detailed"}}"#)
    );
}

fn mapped_table_cases() {
    let cases = [
        (r#"{"temperature": 0.5}"#, r#"{"temperature": 0.5}"#),
        (r#"{"service_tier": null}"#, r#"{"service_tier": null}"#),
        (r#"{"service_tier": "auto"}"#, r#"{"service_tier": "auto"}"#),
        (
            r#"{"service_tier": "default"}"#,
            r#"{"service_tier": "default"}"#,
        ),
        (
            r#"{"max_output_tokens": 8}"#,
            r#"{"max_output_tokens": 16}"#,
        ),
        (
            r#"{"max_output_tokens": 64}"#,
            r#"{"max_output_tokens": 64}"#,
        ),
    ];
    for (params, expected) in cases {
        assert_eq!(
            support::mapped(MODEL, params, false),
            support::object(expected),
            "params {params}"
        );
    }
}
