#![cfg(test)]

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use rstest::rstest;

use super::{
    MaxResultChars, MinCharsToEvaluate, RelevanceThreshold, TypeSafeGuardrailConfigModel,
    TypeSafeSettings,
};
use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::DROPPED_RESULT_TEXT;

const FULL_CONFIG: &str = r#"{"api_base":"https://example.test","api_key":"k","model":"jev-x","unreachable_fallback":"fail_closed","optional_params":{"relevance_threshold":0.7,"min_chars_to_evaluate":50,"max_result_chars_in_state":900}}"#;

fn model(text: &str) -> TypeSafeGuardrailConfigModel {
    serde_json::from_str(text).expect("config model")
}

#[test]
fn typesafe_config_defaults_match_litellm() {
    let settings = TypeSafeSettings::default();
    assert_eq!(settings.api_base, "https://api.typesafe.ai");
    assert_eq!(settings.model, "jev-latest");
    assert_eq!(settings.relevance_threshold, RelevanceThreshold(0.2));
    assert_eq!(settings.min_chars_to_evaluate, MinCharsToEvaluate(200));
    assert_eq!(settings.max_result_chars_in_state, MaxResultChars(4000));
    assert_eq!(settings.unreachable_fallback, UnreachableFallback::FailOpen);
}

#[test]
fn typesafe_config_defaults_come_from_an_empty_model() {
    assert_eq!(
        TypeSafeGuardrailConfigModel::default().resolve_settings(),
        TypeSafeSettings::default()
    );
}

#[test]
fn typesafe_config_serializes_the_seven_parameters() {
    let serialized = serde_json::to_string(&model(FULL_CONFIG)).expect("serialize");
    assert_eq!(
        serialized,
        r#"{"optional_params":{"relevance_threshold":0.7,"min_chars_to_evaluate":50,"max_result_chars_in_state":900},"api_key":"k","api_base":"https://example.test","model":"jev-x","unreachable_fallback":"fail_closed"}"#
    );
}

#[test]
fn typesafe_config_resolves_every_parameter() {
    let settings = model(FULL_CONFIG).resolve_settings();
    assert_eq!(settings.api_base, "https://example.test");
    assert_eq!(settings.model, "jev-x");
    assert_eq!(settings.relevance_threshold, RelevanceThreshold(0.7));
    assert_eq!(settings.min_chars_to_evaluate, MinCharsToEvaluate(50));
    assert_eq!(settings.max_result_chars_in_state, MaxResultChars(900));
    assert_eq!(
        settings.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn typesafe_config_trims_trailing_api_base_slashes() {
    let settings = model(r#"{"api_base":"https://example.test///"}"#).resolve_settings();
    assert_eq!(settings.api_base, "https://example.test");
}

#[rstest]
#[case::top_level_nulls(
    r#"{"api_base":null,"api_key":null,"model":null,"unreachable_fallback":null,"optional_params":null}"#
)]
#[case::optional_param_nulls(
    r#"{"optional_params":{"relevance_threshold":null,"min_chars_to_evaluate":null,"max_result_chars_in_state":null}}"#
)]
fn typesafe_config_takes_defaults_for_null_values(#[case] text: &str) {
    assert_eq!(model(text).resolve_settings(), TypeSafeSettings::default());
}

#[rstest]
#[case::threshold_above(
    r#"{"optional_params":{"relevance_threshold":1.5}}"#,
    "relevance_threshold"
)]
#[case::threshold_below(
    r#"{"optional_params":{"relevance_threshold":-0.1}}"#,
    "relevance_threshold"
)]
#[case::min_chars_negative(
    r#"{"optional_params":{"min_chars_to_evaluate":-1}}"#,
    "min_chars_to_evaluate"
)]
#[case::max_chars_zero(
    r#"{"optional_params":{"max_result_chars_in_state":0}}"#,
    "max_result_chars_in_state"
)]
fn typesafe_config_rejects_out_of_range_values(#[case] text: &str, #[case] field: &str) {
    let error = serde_json::from_str::<TypeSafeGuardrailConfigModel>(text)
        .err()
        .expect("invalid");
    assert!(error.to_string().contains(field), "{error}");
}

#[test]
fn typesafe_config_rejects_unknown_keys() {
    assert!(serde_json::from_str::<TypeSafeGuardrailConfigModel>(r#"{"unknown":1}"#).is_err());
    assert!(
        serde_json::from_str::<TypeSafeGuardrailConfigModel>(
            r#"{"optional_params":{"unknown":1}}"#
        )
        .is_err()
    );
}

#[test]
fn typesafe_ui_friendly_name_matches_litellm() {
    assert_eq!(
        TypeSafeGuardrailConfigModel::ui_friendly_name(),
        "TypeSafe (Jev) Compaction"
    );
}

#[test]
fn typesafe_dropped_result_marker_matches_litellm() {
    assert_eq!(
        DROPPED_RESULT_TEXT,
        "[Tool result removed by TypeSafe compaction: judged no longer relevant to the current task]"
    );
}
