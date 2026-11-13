#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;

use super::{PresidioArgs, PresidioRole, initialize_presidio};
use crate::guardrail_initializers::error::InitializerError;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(mode: &str, extras: &str) -> Result<Vec<PresidioArgs>, InitializerError> {
    let body = format!(r#"{{"guardrail": "presidio", "mode": {mode}{extras}}}"#);
    initialize_presidio(&params(&body), Some("presidio-check"))
}

#[test]
fn default_scope_builds_the_input_scan_and_the_output_mask() {
    let callbacks = build(r#""pre_call""#, "").unwrap();
    assert_eq!(callbacks.len(), 2);
    let input = &callbacks[0];
    assert_eq!(input.role, PresidioRole::Scan);
    assert_eq!(input.output_parse_pii, None);
    assert!(!input.apply_to_output);
    assert!(!input.mask_response_content);
    assert_eq!(
        input.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    let mask = &callbacks[1];
    assert_eq!(mask.role, PresidioRole::Scan);
    assert_eq!(mask.output_parse_pii, Some(false));
    assert!(mask.apply_to_output);
    assert!(mask.mask_response_content);
    assert_eq!(
        mask.event_hook,
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
}

#[rstest]
#[case::input("input", 1, false)]
#[case::output("output", 1, true)]
#[case::both("both", 2, false)]
fn filter_scope_picks_the_callbacks(#[case] scope: &str, #[case] count: usize, #[case] mask: bool) {
    let callbacks = build(
        r#""pre_call""#,
        &format!(r#", "presidio_filter_scope": "{scope}""#),
    )
    .unwrap();
    assert_eq!(callbacks.len(), count);
    assert_eq!(callbacks[0].apply_to_output, mask);
}

#[test]
fn output_parse_pii_adds_the_restore_callback_after_the_input_scan() {
    let callbacks = build(
        r#""pre_call""#,
        r#", "presidio_filter_scope": "input", "output_parse_pii": true"#,
    )
    .unwrap();
    assert_eq!(callbacks.len(), 2);
    assert_eq!(callbacks[0].role, PresidioRole::Scan);
    assert_eq!(callbacks[0].output_parse_pii, Some(true));
    let restore = &callbacks[1];
    assert_eq!(restore.role, PresidioRole::Restore);
    assert_eq!(restore.output_parse_pii, Some(true));
    assert!(!restore.apply_to_output);
    assert!(!restore.mask_response_content);
    assert_eq!(
        restore.event_hook,
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
}

#[test]
fn restore_with_an_mcp_only_mode_keeps_the_post_call_hook() {
    let callbacks = build(
        r#""pre_mcp_call""#,
        r#", "presidio_filter_scope": "input", "output_parse_pii": true"#,
    )
    .unwrap();
    assert_eq!(callbacks.len(), 2);
    assert_eq!(callbacks[1].role, PresidioRole::Restore);
    assert_eq!(
        callbacks[1].event_hook,
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
}

#[test]
fn both_with_output_parse_pii_orders_scan_restore_and_mask() {
    let callbacks = build(
        r#""pre_call""#,
        r#", "presidio_filter_scope": "both", "output_parse_pii": true"#,
    )
    .unwrap();
    assert_eq!(
        callbacks
            .iter()
            .map(|callback| callback.role)
            .collect::<Vec<_>>(),
        [
            PresidioRole::Scan,
            PresidioRole::Restore,
            PresidioRole::Scan
        ]
    );
    assert!(callbacks[2].apply_to_output);
    assert!(callbacks[2].mask_response_content);
    assert_eq!(callbacks[2].output_parse_pii, Some(false));
}

#[test]
fn output_parse_pii_does_not_build_a_restore_for_output_only_scope() {
    let callbacks = build(
        r#""post_call""#,
        r#", "presidio_filter_scope": "output", "output_parse_pii": true"#,
    )
    .unwrap();
    assert_eq!(callbacks.len(), 1);
    assert_eq!(callbacks[0].role, PresidioRole::Scan);
    assert!(callbacks[0].apply_to_output);
}

#[test]
fn mcp_only_mode_defaults_to_the_input_scope() {
    let callbacks = build(r#""pre_mcp_call""#, "").unwrap();
    assert_eq!(callbacks.len(), 1);
    assert_eq!(callbacks[0].role, PresidioRole::Scan);
    assert!(!callbacks[0].apply_to_output);
    assert_eq!(
        callbacks[0].event_hook,
        LitellmParamsMode::One("pre_mcp_call".to_owned())
    );
}

#[test]
fn mcp_only_mode_with_an_explicit_scope_keeps_mcp_output_hooks() {
    let callbacks = build(r#""pre_mcp_call""#, r#", "presidio_filter_scope": "both""#).unwrap();
    assert_eq!(callbacks.len(), 2);
    assert_eq!(
        callbacks[0].event_hook,
        LitellmParamsMode::One("pre_mcp_call".to_owned())
    );
    assert_eq!(
        callbacks[1].event_hook,
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
}

#[test]
fn tagged_mode_mask_keeps_mcp_only_tags_empty_without_an_explicit_scope() {
    let callbacks = build(
        r#"{"tags": {"chat": "pre_call", "mcp": "pre_mcp_call"}}"#,
        "",
    )
    .unwrap();
    assert_eq!(callbacks.len(), 2);
    assert_eq!(
        callbacks[1].event_hook,
        LitellmParamsMode::Tagged(Mode {
            tags: BTreeMap::from([
                (
                    "chat".to_owned(),
                    ModeTarget::Many(vec!["post_call".to_owned()]),
                ),
                ("mcp".to_owned(), ModeTarget::Many(Vec::new())),
            ]),
            default: None,
        })
    );
}

const LISTED: &str = r#",
    "presidio_filter_scope": "input",
    "presidio_ad_hoc_recognizers": "/tmp/recognizers.json",
    "mock_redacted_text": {"text": "masked", "items": []},
    "default_on": true,
    "pii_entities_config": {"PHONE_NUMBER": "MASK", "CREDIT_CARD": "BLOCK"},
    "presidio_score_thresholds": {"PERSON": 0.4},
    "presidio_analyzer_api_base": "https://analyzer",
    "presidio_anonymizer_api_base": "https://anonymizer",
    "presidio_language": "de",
    "presidio_entities_deny_list": ["US_DRIVER_LICENSE"],
    "timeout": 6.0,
    "presidio_analyze_chunk_size_bytes": 1000
    "#;

#[test]
fn presidio_args_copy_the_recognizer_and_mock_fields() {
    let callbacks = build(r#""pre_call""#, LISTED).unwrap();
    let built = &callbacks[0];
    assert_eq!(built.name, "presidio-check");
    assert_eq!(
        built.presidio_ad_hoc_recognizers.as_deref(),
        Some("/tmp/recognizers.json")
    );
    assert_eq!(
        built.mock_redacted_text.as_ref().unwrap().get("text"),
        Some(&GuardrailValue::Text("masked".to_owned()))
    );
    assert!(built.default_on);
}

#[test]
fn presidio_args_copy_the_entity_configuration() {
    let callbacks = build(r#""pre_call""#, LISTED).unwrap();
    let built = &callbacks[0];
    assert_eq!(
        built
            .pii_entities_config
            .as_ref()
            .unwrap()
            .get("PHONE_NUMBER"),
        Some(&PiiAction::Mask)
    );
    assert_eq!(
        built
            .pii_entities_config
            .as_ref()
            .unwrap()
            .get("CREDIT_CARD"),
        Some(&PiiAction::Block)
    );
    assert_eq!(
        built
            .presidio_score_thresholds
            .as_ref()
            .unwrap()
            .get("PERSON"),
        Some(&0.4_f64)
    );
    assert_eq!(
        built.presidio_entities_deny_list.as_deref(),
        Some(["US_DRIVER_LICENSE".to_owned()].as_slice())
    );
}

#[test]
fn presidio_args_copy_the_endpoints_and_limits() {
    let callbacks = build(r#""pre_call""#, LISTED).unwrap();
    let built = &callbacks[0];
    assert_eq!(
        built.presidio_analyzer_api_base.as_deref(),
        Some("https://analyzer")
    );
    assert_eq!(
        built.presidio_anonymizer_api_base.as_deref(),
        Some("https://anonymizer")
    );
    assert_eq!(built.presidio_language.as_deref(), Some("de"));
    assert_eq!(built.timeout, Some(6.0_f64));
    assert_eq!(built.analyze_chunk_size_bytes, Some(1000));
}

#[test]
fn presidio_language_defaults_to_en_and_null_stays_absent() {
    let default_language = build(r#""pre_call""#, r#", "presidio_filter_scope": "input""#).unwrap();
    assert_eq!(default_language[0].presidio_language.as_deref(), Some("en"));
    let absent = build(
        r#""pre_call""#,
        r#", "presidio_filter_scope": "input", "presidio_language": null"#,
    )
    .unwrap();
    assert_eq!(absent[0].presidio_language, None);
}

#[test]
fn negative_chunk_size_reads_as_absent_and_zero_is_kept() {
    let negative = build(
        r#""pre_call""#,
        r#", "presidio_filter_scope": "input", "presidio_analyze_chunk_size_bytes": -5"#,
    )
    .unwrap();
    assert_eq!(negative[0].analyze_chunk_size_bytes, None);
    let zero = build(
        r#""pre_call""#,
        r#", "presidio_filter_scope": "input", "presidio_analyze_chunk_size_bytes": 0"#,
    )
    .unwrap();
    assert_eq!(zero[0].analyze_chunk_size_bytes, Some(0));
}

#[rstest]
#[case::unknown_scope(r#", "presidio_filter_scope": "sideways""#, "presidio_filter_scope")]
#[case::scope_wrong_type(r#", "presidio_filter_scope": 1"#, "presidio_filter_scope")]
#[case::bad_entity_action(
    r#", "pii_entities_config": {"PHONE_NUMBER": "DROP"}"#,
    "pii_entities_config"
)]
#[case::bad_threshold(
    r#", "presidio_score_thresholds": {"PERSON": true}"#,
    "presidio_score_thresholds"
)]
#[case::bad_deny_entry(
    r#", "presidio_entities_deny_list": ["US_SSN", 4]"#,
    "presidio_entities_deny_list"
)]
#[case::bad_mock(r#", "mock_redacted_text": ["masked"]"#, "mock_redacted_text")]
fn bad_presidio_extras_name_the_field(#[case] extras: &str, #[case] field: &str) {
    let error = build(r#""pre_call""#, extras).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidParam(name) if name.as_str() == field),
        "unexpected error {error}"
    );
}
