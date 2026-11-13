#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::initialize_qohash;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

#[test]
fn qohash_args_copy_the_listed_fields() {
    let built = initialize_qohash(
        &params(
            r#"{
                "guardrail": "qostodian_nexus",
                "mode": "during_call",
                "api_base": "https://nexus.example",
                "default_on": true,
                "additional_provider_specific_params": {"threshold": 0.5},
                "extra_headers": ["x-caller-header"],
                "timeout": 1.5
            }"#,
        ),
        Some("nexus-check"),
    );
    assert_eq!(built.api_base.as_deref(), Some("https://nexus.example"));
    assert_eq!(built.name, "nexus-check");
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("during_call".to_owned())
    );
    assert!(built.default_on);
    assert_eq!(
        built.provider_params,
        Some(OpaqueParams(BTreeMap::from([(
            "threshold".to_owned(),
            GuardrailValue::Decimal(0.5_f64)
        )])))
    );
    assert_eq!(
        built.extra_headers,
        Some(vec!["x-caller-header".to_owned()])
    );
    assert_eq!(built.timeout, Some(1.5_f64));
}

#[test]
fn qohash_args_leave_absent_settings_unset_and_the_name_empty() {
    let built = initialize_qohash(
        &params(r#"{"guardrail": "qostodian_nexus", "mode": "pre_call"}"#),
        None,
    );
    assert_eq!(built.api_base, None);
    assert_eq!(built.name, "");
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(!built.default_on);
    assert_eq!(built.provider_params, None);
    assert_eq!(built.extra_headers, None);
    assert_eq!(built.timeout, None);
}

#[test]
fn qohash_args_keep_the_mode_lists_litellm_accepts() {
    let built = initialize_qohash(
        &params(r#"{"guardrail": "qostodian_nexus", "mode": ["pre_call", "post_call"]}"#),
        Some("nexus-check"),
    );
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
}
