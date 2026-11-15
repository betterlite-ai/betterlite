#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::initialize_crowdstrike;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn crowdstrike_refuses_a_missing_or_empty_name(#[case] name: Option<&str>) {
    let error = initialize_crowdstrike(&params(""), name).unwrap_err();
    assert!(matches!(
        error,
        ThirdPartyInitAError::MissingCrowdStrikeName
    ));
    assert_eq!(
        error.to_string(),
        "CrowdStrike AIDR guardrail name is required"
    );
}

#[test]
fn crowdstrike_takes_the_defaults_of_an_empty_config() {
    let built = initialize_crowdstrike(&params(""), Some("crowdstrike-guard")).unwrap();
    assert_eq!(built.name, "crowdstrike-guard");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert!(built.fail_on_error);
    assert_eq!(built.streaming_buffer_until_moderated, None);
    assert_eq!(built.streaming_buffer_release_on_scan, None);
    assert_eq!(built.streaming_end_of_stream_only, None);
    assert_eq!(built.streaming_sampling_rate, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn crowdstrike_copies_the_configured_fields() {
    let built = initialize_crowdstrike(
        &params(
            r#",
            "api_base": "https://crowdstrike.test",
            "api_key": "cs-token",
            "fail_on_error": false,
            "timeout": 3,
            "default_on": true
            "#,
        ),
        Some("crowdstrike-guard"),
    )
    .unwrap();
    assert_eq!(built.api_base.as_deref(), Some("https://crowdstrike.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("cs-token"));
    assert!(!built.fail_on_error);
    assert_eq!(built.timeout, Some(3.0_f64));
    assert!(built.default_on);
}

#[rstest]
#[case::buffer_until_moderated("streaming_buffer_until_moderated", [true, false, false])]
#[case::buffer_release_on_scan("streaming_buffer_release_on_scan", [false, true, false])]
#[case::end_of_stream_only("streaming_end_of_stream_only", [false, false, true])]
fn crowdstrike_takes_each_streaming_flag_from_the_streaming_helper(
    #[case] flag: &str,
    #[case] expected: [bool; 3],
) {
    let others = [
        "streaming_buffer_until_moderated",
        "streaming_buffer_release_on_scan",
        "streaming_end_of_stream_only",
    ]
    .map(|name| format!(r#""{name}": {}"#, name == flag));
    let extras = format!(
        r#", "optional_params": {{{}, "streaming_sampling_rate": 3}}"#,
        others.join(", ")
    );
    let built = initialize_crowdstrike(&params(&extras), Some("crowdstrike-guard")).unwrap();
    assert_eq!(
        [
            built.streaming_buffer_until_moderated,
            built.streaming_buffer_release_on_scan,
            built.streaming_end_of_stream_only,
        ],
        expected.map(Some)
    );
    assert_eq!(built.streaming_sampling_rate, Some(3));
}

#[test]
fn crowdstrike_lets_the_optional_params_streaming_flag_win_over_the_top_level() {
    let built = initialize_crowdstrike(
        &params(
            r#",
            "streaming_buffer_until_moderated": false,
            "optional_params": {"streaming_buffer_until_moderated": true}
            "#,
        ),
        Some("crowdstrike-guard"),
    )
    .unwrap();
    assert_eq!(built.streaming_buffer_until_moderated, Some(true));
}

#[test]
fn crowdstrike_refuses_a_streaming_param_the_helper_refuses() {
    let error = initialize_crowdstrike(
        &params(r#", "streaming_sampling_rate": 0"#),
        Some("crowdstrike-guard"),
    )
    .unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::Streaming { .. }));
}

#[test]
fn crowdstrike_checks_the_name_before_the_streaming_params() {
    let error =
        initialize_crowdstrike(&params(r#", "streaming_sampling_rate": 0"#), None).unwrap_err();
    assert!(matches!(
        error,
        ThirdPartyInitAError::MissingCrowdStrikeName
    ));
}
