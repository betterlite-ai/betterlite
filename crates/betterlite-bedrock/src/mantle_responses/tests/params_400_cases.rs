use crate::mantle_responses::error::MantleResponsesError;

use super::support;

const MODEL: &str = "openai.gpt-oss-120b";

const TIER_MESSAGE: &str = "bedrock_mantle does not support service_tier='flex'; the Bedrock Mantle Responses API only accepts 'auto' or 'default'. Set `drop_params: true` (litellm_settings or this deployment's litellm_params) to have LiteLLM drop it, or remove service_tier from the client (Codex CLI sends it when a speed tier is set in ~/.codex/config.toml).";

const SUMMARY_MESSAGE: &str = "bedrock_mantle does not support reasoning.summary='concise' for 'openai.gpt-oss-120b'; the Bedrock Mantle OpenAI Responses path only accepts 'auto'. Set `drop_params: true` (litellm_settings or this deployment's litellm_params) to have LiteLLM drop it, or set `model_reasoning_summary = \"auto\"` in the client (Codex CLI: ~/.codex/config.toml).";

pub(super) fn unsupported_params_400() {
    tier_cases();
    summary_cases();
    ordering_cases();
}

fn message_of(error: &MantleResponsesError) -> String {
    match error {
        MantleResponsesError::UnsupportedParams { message } => message.clone(),
        other => panic!("expected an unsupported params error, got {other:?}"),
    }
}

fn tier_cases() {
    let _guard = support::capture().1;
    let error = support::rejected(MODEL, r#"{"service_tier": "flex"}"#, false);
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(message_of(&error), TIER_MESSAGE);
    let priority = support::rejected(MODEL, r#"{"service_tier": "priority"}"#, false);
    assert_eq!(
        message_of(&priority),
        "bedrock_mantle does not support service_tier='priority'; the Bedrock Mantle Responses API only accepts 'auto' or 'default'. Set `drop_params: true` (litellm_settings or this deployment's litellm_params) to have LiteLLM drop it, or remove service_tier from the client (Codex CLI sends it when a speed tier is set in ~/.codex/config.toml)."
    );
    let numeric = support::rejected(MODEL, r#"{"service_tier": 3}"#, false);
    assert!(message_of(&numeric).starts_with("bedrock_mantle does not support service_tier=3;"));
}

fn summary_cases() {
    let _guard = support::capture().1;
    let error = support::rejected(MODEL, r#"{"reasoning": {"summary": "concise"}}"#, false);
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(message_of(&error), SUMMARY_MESSAGE);
    let detailed = support::rejected(MODEL, r#"{"reasoning": {"summary": "detailed"}}"#, false);
    assert_eq!(
        message_of(&detailed),
        "bedrock_mantle does not support reasoning.summary='detailed' for 'openai.gpt-oss-120b'; the Bedrock Mantle OpenAI Responses path only accepts 'auto'. Set `drop_params: true` (litellm_settings or this deployment's litellm_params) to have LiteLLM drop it, or set `model_reasoning_summary = \"auto\"` in the client (Codex CLI: ~/.codex/config.toml)."
    );
}

fn ordering_cases() {
    let _guard = support::capture().1;
    let both = support::rejected(
        MODEL,
        r#"{"service_tier": "flex", "reasoning": {"summary": "concise"}}"#,
        false,
    );
    assert_eq!(message_of(&both), TIER_MESSAGE);
    let tier_only = support::rejected(
        MODEL,
        r#"{"service_tier": "flex", "reasoning": {"summary": "auto"}}"#,
        false,
    );
    assert_eq!(message_of(&tier_only), TIER_MESSAGE);
    let summary_only = support::rejected(
        MODEL,
        r#"{"service_tier": "auto", "reasoning": {"summary": "concise"}}"#,
        false,
    );
    assert_eq!(message_of(&summary_only), SUMMARY_MESSAGE);
}
