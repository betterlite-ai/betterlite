#![cfg(test)]

use super::MetadataFields;
use super::identity::Mode;
use super::levels::ReasoningEffort;
use rstest::rstest;

#[rstest]
#[case::limits(
    r#"{"litellm_provider":"openai","max_input_tokens":128000,"max_output_tokens":16384}"#
)]
#[case::provider_mode(r#"{"litellm_provider":"anthropic","mode":"chat"}"#)]
#[case::modalities_regions(
    r#"{"litellm_provider":"vertex_ai","supported_modalities":["text","image"],"supported_regions":["global","us-east1"]}"#
)]
#[case::reasoning_levels(
    r#"{"litellm_provider":"anthropic","default_reasoning_effort":"low","reasoning_effort_levels":["low","medium","high"]}"#
)]
#[case::deprecation_and_source(
    r#"{"litellm_provider":"openai","deprecation_date":"2025-06-30","source":"https://openai.com/pricing"}"#
)]
fn model_metadata_types_model_field_tests_behavior(#[case] source: &str) {
    let fields: MetadataFields = serde_json::from_str(source).unwrap();
    assert!(!fields.litellm_provider.is_empty());
}

#[test]
fn limits_round_trip_as_integers() {
    let fields: MetadataFields =
        serde_json::from_str(r#"{"litellm_provider":"openai","max_input_tokens":128000}"#).unwrap();
    assert_eq!(fields.max_input_tokens, Some(128_000));
}

#[test]
fn provider_mode_decodes_the_closed_mode_set() {
    let fields: MetadataFields =
        serde_json::from_str(r#"{"litellm_provider":"openai","mode":"embedding"}"#).unwrap();
    assert_eq!(fields.mode, Some(Mode::Embedding));
}

#[test]
fn reasoning_levels_share_one_superset_enum() {
    let fields: MetadataFields = serde_json::from_str(
        r#"{"litellm_provider":"bedrock","bedrock_output_config_effort_ceiling":"xhigh"}"#,
    )
    .unwrap();
    assert_eq!(fields.bedrock_output_ceiling, Some(ReasoningEffort::Xhigh));
}

#[test]
fn deprecation_date_parses_as_a_calendar_date() {
    let fields: MetadataFields =
        serde_json::from_str(r#"{"litellm_provider":"openai","deprecation_date":"2025-06-30"}"#)
            .unwrap();
    assert_eq!(
        fields.deprecation_date,
        Some(chrono::NaiveDate::from_ymd_opt(2025, 6, 30).unwrap())
    );
}

#[test]
fn a_non_date_deprecation_value_fails_to_parse() {
    serde_json::from_str::<MetadataFields>(
        r#"{"litellm_provider":"openai","deprecation_date":"date when the model becomes deprecated"}"#,
    )
    .unwrap_err();
}

#[test]
fn catalog_notes_type_the_non_price_metadata_subkeys() {
    let fields: MetadataFields = serde_json::from_str(
        r#"{"litellm_provider":"together_ai","metadata":{"successor":"together_ai/new-model","supports_tool_choice":true}}"#,
    )
    .unwrap();
    assert_eq!(
        fields.notes.successor,
        Some("together_ai/new-model".to_owned())
    );
    assert_eq!(fields.notes.supports_tool_choice, Some(true));
}
