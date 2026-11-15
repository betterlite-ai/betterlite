#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::initialize_grayswan;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn grayswan_refuses_a_missing_or_empty_name(#[case] name: Option<&str>) {
    let error = initialize_grayswan(&params(""), name).unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingGraySwanName));
    assert_eq!(
        error.to_string(),
        "Gray Swan guardrail requires a guardrail_name"
    );
}

#[test]
fn grayswan_takes_the_defaults_of_an_empty_config() {
    let built = initialize_grayswan(&params(""), Some("gray-guard")).unwrap();
    assert_eq!(built.name, "gray-guard");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.on_flagged_action.as_deref(), Some("monitor"));
    assert_eq!(built.violation_threshold, None);
    assert_eq!(built.reasoning_mode, None);
    assert_eq!(built.categories, None);
    assert_eq!(built.policy_id, None);
    assert!(!built.streaming_end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, 5);
    assert_eq!(built.fail_open, None);
    assert_eq!(built.guardrail_timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn grayswan_copies_the_optional_params() {
    let built = initialize_grayswan(
        &params(
            r#",
            "api_key": "gray-key",
            "api_base": "https://gray.test",
            "default_on": true,
            "optional_params": {
                "on_flagged_action": "passthrough",
                "violation_threshold": 0.7,
                "reasoning_mode": "hybrid",
                "categories": {"violence": "no violence"},
                "policy_id": "policy-1",
                "streaming_end_of_stream_only": true,
                "streaming_sampling_rate": 3,
                "fail_open": false,
                "guardrail_timeout": 12.5
            }
            "#,
        ),
        Some("gray-guard"),
    )
    .unwrap();
    assert_eq!(exposed(built.api_key.as_ref()), Some("gray-key"));
    assert_eq!(built.api_base.as_deref(), Some("https://gray.test"));
    assert_eq!(built.on_flagged_action.as_deref(), Some("passthrough"));
    assert_eq!(built.violation_threshold, Some(0.7_f64));
    assert_eq!(built.reasoning_mode.as_deref(), Some("hybrid"));
    assert_eq!(
        built.categories,
        Some([("violence".to_owned(), "no violence".to_owned())].into())
    );
    assert_eq!(built.policy_id.as_deref(), Some("policy-1"));
    assert!(built.streaming_end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, 3);
    assert_eq!(built.fail_open, Some(false));
    assert_eq!(built.guardrail_timeout, Some(12.5_f64));
    assert!(built.default_on);
}

#[test]
fn grayswan_falls_back_to_the_top_level_params() {
    let built = initialize_grayswan(
        &params(
            r#",
            "on_flagged_action": "block",
            "violation_threshold": 0.2,
            "reasoning_mode": "off",
            "policy_id": "top-policy",
            "streaming_end_of_stream_only": true,
            "streaming_sampling_rate": 8,
            "fail_open": false,
            "guardrail_timeout": 4
            "#,
        ),
        Some("gray-guard"),
    )
    .unwrap();
    assert_eq!(built.on_flagged_action.as_deref(), Some("block"));
    assert_eq!(built.violation_threshold, Some(0.2_f64));
    assert_eq!(built.reasoning_mode.as_deref(), Some("off"));
    assert_eq!(built.policy_id.as_deref(), Some("top-policy"));
    assert!(built.streaming_end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, 8);
    assert_eq!(built.fail_open, Some(false));
    assert_eq!(built.guardrail_timeout, Some(4.0_f64));
}

#[test]
fn grayswan_lets_an_optional_params_value_win_over_the_top_level() {
    let built = initialize_grayswan(
        &params(
            r#",
            "on_flagged_action": "monitor",
            "violation_threshold": 0.2,
            "streaming_sampling_rate": 8,
            "optional_params": {
                "on_flagged_action": "block",
                "violation_threshold": 0.9,
                "streaming_sampling_rate": 2
            }
            "#,
        ),
        Some("gray-guard"),
    )
    .unwrap();
    assert_eq!(built.on_flagged_action.as_deref(), Some("block"));
    assert_eq!(built.violation_threshold, Some(0.9_f64));
    assert_eq!(built.streaming_sampling_rate, 2);
}

#[test]
fn grayswan_falls_back_to_the_top_level_for_a_null_optional_entry() {
    let built = initialize_grayswan(
        &params(
            r#",
            "on_flagged_action": "passthrough",
            "violation_threshold": 0.4,
            "guardrail_timeout": 6,
            "optional_params": {
                "on_flagged_action": null,
                "violation_threshold": null,
                "guardrail_timeout": null
            }
            "#,
        ),
        Some("gray-guard"),
    )
    .unwrap();
    assert_eq!(built.on_flagged_action.as_deref(), Some("passthrough"));
    assert_eq!(built.violation_threshold, Some(0.4_f64));
    assert_eq!(built.guardrail_timeout, Some(6.0_f64));
}

#[rstest]
#[case::no_optional_params("", Some("monitor"))]
#[case::empty_optional_params(r#", "optional_params": {}"#, Some("block"))]
#[case::other_optional_entry(r#", "optional_params": {"fail_open": true}"#, Some("block"))]
#[case::null_optional_entry(r#", "optional_params": {"on_flagged_action": null}"#, Some("monitor"))]
#[case::null_optional_params(r#", "optional_params": null"#, Some("monitor"))]
#[case::null_optional_entry_and_top_level(
    r#", "on_flagged_action": null, "optional_params": {"on_flagged_action": null}"#,
    None
)]
#[case::top_level_null(r#", "on_flagged_action": null"#, None)]
#[case::top_level_set(r#", "on_flagged_action": "passthrough""#, Some("passthrough"))]
#[case::optional_over_top_level(
    r#", "on_flagged_action": "monitor", "optional_params": {"on_flagged_action": "passthrough"}"#,
    Some("passthrough")
)]
fn grayswan_resolves_the_flagged_action_in_the_order_litellm_reads_it(
    #[case] extras: &str,
    #[case] expected: Option<&str>,
) {
    let built = initialize_grayswan(&params(extras), Some("gray-guard")).unwrap();
    assert_eq!(built.on_flagged_action.as_deref(), expected);
}

#[rstest]
#[case::absent("", false, 5)]
#[case::false_and_zero(
    r#", "streaming_end_of_stream_only": false, "streaming_sampling_rate": 0"#,
    false,
    5
)]
#[case::true_and_one(
    r#", "streaming_end_of_stream_only": true, "streaming_sampling_rate": 1"#,
    true,
    1
)]
#[case::null_values(
    r#", "streaming_end_of_stream_only": null, "streaming_sampling_rate": null"#,
    false,
    5
)]
#[case::optional_zero(
    r#", "optional_params": {"streaming_end_of_stream_only": false, "streaming_sampling_rate": 0}"#,
    false,
    5
)]
#[case::negative_rate_passes_through(
    r#", "streaming_sampling_rate": -2"#,
    false,
    -2
)]
#[case::optional_values(
    r#", "optional_params": {"streaming_end_of_stream_only": true, "streaming_sampling_rate": 12}"#,
    true,
    12
)]
#[case::optional_zero_over_top_level(
    r#", "streaming_sampling_rate": 7, "optional_params": {"streaming_sampling_rate": 0}"#,
    false,
    5
)]
fn grayswan_applies_the_or_defaults_of_the_two_streaming_fields(
    #[case] extras: &str,
    #[case] end_of_stream_only: bool,
    #[case] sampling_rate: i64,
) {
    let built = initialize_grayswan(&params(extras), Some("gray-guard")).unwrap();
    assert_eq!(built.streaming_end_of_stream_only, end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, sampling_rate);
}

#[test]
fn grayswan_reads_the_timeout_from_guardrail_timeout_only() {
    let built = initialize_grayswan(&params(r#", "timeout": 30"#), Some("gray-guard")).unwrap();
    assert_eq!(built.guardrail_timeout, None);
}

#[test]
fn grayswan_reads_categories_from_optional_params_only() {
    let built = initialize_grayswan(
        &params(
            r#",
            "categories": [{"category": "harmful_violence", "action": "BLOCK"}],
            "optional_params": {}
            "#,
        ),
        Some("gray-guard"),
    )
    .unwrap();
    assert_eq!(built.categories, None);
}

#[rstest]
#[case::threshold_text(
    r#", "optional_params": {"violation_threshold": "high"}"#,
    "violation_threshold"
)]
#[case::fail_open_text(r#", "fail_open": "no""#, "fail_open")]
#[case::categories_list(r#", "optional_params": {"categories": ["a"]}"#, "categories")]
#[case::category_value_number(r#", "optional_params": {"categories": {"a": 1}}"#, "categories")]
#[case::rate_text(r#", "streaming_sampling_rate": "often""#, "streaming_sampling_rate")]
#[case::eos_number(
    r#", "streaming_end_of_stream_only": 1"#,
    "streaming_end_of_stream_only"
)]
#[case::action_number(
    r#", "optional_params": {"on_flagged_action": 3}"#,
    "on_flagged_action"
)]
#[case::top_level_action_number(r#", "on_flagged_action": 3"#, "on_flagged_action")]
#[case::timeout_text(r#", "guardrail_timeout": "slow""#, "guardrail_timeout")]
#[case::optional_params_text(r#", "optional_params": "text""#, "optional_params")]
fn grayswan_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_grayswan(&params(extras), Some("gray-guard")).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}
