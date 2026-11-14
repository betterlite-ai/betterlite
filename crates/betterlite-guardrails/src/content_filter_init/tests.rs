#![cfg(test)]

use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterSeverity,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;

use super::error::ContentFilterInitError;
use super::initialize_content_filter;

const MINIMAL: &str = r#"{"guardrail": "litellm_content_filter", "mode": "pre_call"}"#;

const EVERY_FIELD: &str = r#"{
    "guardrail": "litellm_content_filter",
    "mode": "during_call",
    "patterns": [
        {"pattern_type": "prebuilt", "pattern_name": "us_ssn", "action": "MASK"}
    ],
    "blocked_words": [
        {"keyword": "secret", "action": "BLOCK", "description": "a secret"}
    ],
    "blocked_words_file": "words.yaml",
    "categories": [
        {"category": "harmful_self_harm", "enabled": true, "action": "BLOCK", "severity_threshold": "high"}
    ],
    "severity_threshold": "low",
    "default_on": true,
    "end_session_after_n_fails": 3,
    "on_violation": "warn",
    "realtime_violation_message": "stopped",
    "image_model": "gpt-4o-mini",
    "competitor_intent_config": {
        "competitor_intent_type": "airline",
        "brand_self": ["betterlite"],
        "competitors": ["rival"],
        "locations": ["paris"]
    }
}"#;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn refuses_a_missing_or_empty_name(#[case] name: Option<&str>) {
    let error = initialize_content_filter(&params(MINIMAL), name, None, None, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Content Filter: guardrail_name is required"
    );
    assert!(matches!(error, ContentFilterInitError::MissingName));
}

#[test]
fn copies_the_guardrail_record_fields() {
    let built = initialize_content_filter(
        &params(MINIMAL),
        Some("content-filter"),
        Some("guard-1"),
        Some("policy-template"),
        true,
    )
    .unwrap();
    assert_eq!(built.name, "content-filter");
    assert_eq!(built.guardrail_id.as_deref(), Some("guard-1"));
    assert_eq!(built.policy_template.as_deref(), Some("policy-template"));
    assert!(built.router_present);
}

#[test]
fn copies_the_configured_filter_fields() {
    let built = initialize_content_filter(
        &params(EVERY_FIELD),
        Some("content-filter"),
        None,
        None,
        false,
    )
    .unwrap();
    let patterns = built.patterns.as_ref().unwrap();
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].pattern_type, "prebuilt");
    assert_eq!(patterns[0].pattern_name.as_deref(), Some("us_ssn"));
    assert_eq!(patterns[0].action, ContentFilterAction::Mask);
    assert_eq!(built.blocked_words_file.as_deref(), Some("words.yaml"));
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("during_call".to_owned())
    );
    assert!(built.default_on);
    assert_eq!(built.severity_threshold, "low");
}

#[test]
fn copies_the_blocked_words_and_categories() {
    let built = initialize_content_filter(
        &params(EVERY_FIELD),
        Some("content-filter"),
        None,
        None,
        false,
    )
    .unwrap();
    let words = built.blocked_words.as_ref().unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].keyword, "secret");
    assert_eq!(words[0].action, ContentFilterAction::Block);
    assert_eq!(words[0].description.as_deref(), Some("a secret"));
    let categories = built.categories.as_ref().unwrap();
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].category, "harmful_self_harm");
    assert!(categories[0].enabled);
    assert_eq!(categories[0].action, ContentFilterAction::Block);
    assert_eq!(
        categories[0].severity_threshold,
        ContentFilterSeverity::High
    );
}

#[test]
fn copies_the_extra_and_session_fields() {
    let built = initialize_content_filter(
        &params(EVERY_FIELD),
        Some("content-filter"),
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(built.image_model.as_deref(), Some("gpt-4o-mini"));
    let competitor = built.competitor_intent_config.as_ref().unwrap();
    assert_eq!(
        competitor.0["competitor_intent_type"],
        GuardrailValue::Text("airline".to_owned())
    );
    assert_eq!(
        competitor.0["locations"],
        GuardrailValue::Array(vec![GuardrailValue::Text("paris".to_owned())])
    );
    assert_eq!(built.end_session_after_n_fails, Some(3));
    assert_eq!(built.on_violation, Some(OnViolation::Warn));
    assert_eq!(built.realtime_violation_message.as_deref(), Some("stopped"));
}

#[test]
fn defaults_the_severity_and_the_default_on_flag() {
    let built =
        initialize_content_filter(&params(MINIMAL), Some("safety"), None, None, false).unwrap();
    assert_eq!(built.name, "safety");
    assert_eq!(built.guardrail_id, None);
    assert_eq!(built.policy_template, None);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(!built.default_on);
    assert_eq!(built.severity_threshold, "medium");
    assert!(!built.router_present);
}

#[test]
fn leaves_the_other_optional_fields_absent() {
    let built =
        initialize_content_filter(&params(MINIMAL), Some("safety"), None, None, false).unwrap();
    assert!(built.patterns.is_none());
    assert!(built.blocked_words.is_none());
    assert_eq!(built.blocked_words_file, None);
    assert!(built.categories.is_none());
    assert_eq!(built.image_model, None);
    assert_eq!(built.competitor_intent_config, None);
    assert_eq!(built.end_session_after_n_fails, None);
    assert_eq!(built.on_violation, None);
    assert_eq!(built.realtime_violation_message, None);
}

#[test]
fn keeps_an_explicitly_empty_severity() {
    let source =
        r#"{"guardrail": "litellm_content_filter", "mode": "pre_call", "severity_threshold": ""}"#;
    let built =
        initialize_content_filter(&params(source), Some("safety"), None, None, false).unwrap();
    assert_eq!(built.severity_threshold, "");
}

#[test]
fn treats_a_null_extra_as_absent() {
    let source = r#"{
        "guardrail": "litellm_content_filter",
        "mode": "pre_call",
        "image_model": null,
        "competitor_intent_config": null
    }"#;
    let built =
        initialize_content_filter(&params(source), Some("safety"), None, None, false).unwrap();
    assert_eq!(built.image_model, None);
    assert_eq!(built.competitor_intent_config, None);
}

#[rstest]
#[case::image_model(
    r#"{"guardrail": "litellm_content_filter", "mode": "pre_call", "image_model": 5}"#,
    "image_model"
)]
#[case::competitor_intent_config(
    r#"{"guardrail": "litellm_content_filter", "mode": "pre_call", "competitor_intent_config": ["airline"]}"#,
    "competitor_intent_config"
)]
fn refuses_a_wrong_typed_extra(#[case] source: &str, #[case] field: &str) {
    let error =
        initialize_content_filter(&params(source), Some("safety"), None, None, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the guardrail parameter {field} has an invalid value")
    );
}
