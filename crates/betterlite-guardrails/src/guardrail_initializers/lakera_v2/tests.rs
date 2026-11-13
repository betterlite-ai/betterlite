#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_lakera_v2;
use crate::guardrail_initializers::error::InitializerError;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(extras: &str) -> Result<super::LakeraV2Args, InitializerError> {
    let body = format!(r#"{{"guardrail": "lakera_v2", "mode": "pre_call"{extras}}}"#);
    initialize_lakera_v2(&params(&body), Some("v2-check"))
}

const LISTED: &str = r#",
    "api_base": "https://v2.example",
    "api_key": "v2-key",
    "project_id": "project-1",
    "payload": false,
    "breakdown": false,
    "metadata": {"team": "blue"},
    "dev_info": false,
    "on_flagged": "monitor",
    "skip_system_message_in_guardrail": true,
    "skip_tool_message_in_guardrail": false,
    "advisory_system_message": "careful",
    "default_on": true,
    "timeout": 3.0
    "#;

#[test]
fn lakera_v2_args_copy_the_connection_fields() {
    let built = build(LISTED).unwrap();
    assert_eq!(built.name, "v2-check");
    assert_eq!(built.api_base.as_deref(), Some("https://v2.example"));
    assert_eq!(built.api_key.as_ref().unwrap().expose_secret(), "v2-key");
    assert_eq!(built.project_id.as_deref(), Some("project-1"));
    assert_eq!(built.payload, Some(false));
    assert_eq!(built.breakdown, Some(false));
    assert_eq!(built.dev_info, Some(false));
}

#[test]
fn lakera_v2_args_copy_the_policy_fields() {
    let built = build(LISTED).unwrap();
    assert_eq!(
        built.metadata.as_ref().unwrap().get("team"),
        Some(&GuardrailValue::Text("blue".to_owned()))
    );
    assert_eq!(built.on_flagged, Some(LakeraOnFlagged::Monitor));
    assert_eq!(built.skip_system_message, Some(true));
    assert_eq!(built.skip_tool_message_in_guardrail, Some(false));
    assert_eq!(built.advisory_system_message.as_deref(), Some("careful"));
    assert!(built.default_on);
    assert_eq!(built.timeout, Some(3.0_f64));
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn lakera_v2_args_take_the_declared_defaults() {
    let built = build("").unwrap();
    assert_eq!(built.payload, Some(true));
    assert_eq!(built.breakdown, Some(true));
    assert_eq!(built.dev_info, Some(true));
    assert_eq!(built.on_flagged, Some(LakeraOnFlagged::Block));
    assert_eq!(built.project_id, None);
    assert_eq!(built.metadata, None);
    assert_eq!(built.skip_system_message, None);
    assert_eq!(built.skip_tool_message_in_guardrail, None);
    assert_eq!(built.advisory_system_message, None);
}

#[test]
fn lakera_v2_nulls_stay_absent() {
    let built = build(
        r#",
        "payload": null,
        "breakdown": null,
        "dev_info": null,
        "on_flagged": null,
        "project_id": null
        "#,
    )
    .unwrap();
    assert_eq!(built.payload, None);
    assert_eq!(built.breakdown, None);
    assert_eq!(built.dev_info, None);
    assert_eq!(built.on_flagged, None);
    assert_eq!(built.project_id, None);
}

#[rstest]
#[case::unknown_flag(r#", "on_flagged": "blockk""#, "on_flagged")]
#[case::flag_wrong_type(r#", "on_flagged": 3"#, "on_flagged")]
#[case::payload_wrong_type(r#", "payload": "yes""#, "payload")]
#[case::metadata_wrong_type(r#", "metadata": ["team"]"#, "metadata")]
fn bad_lakera_v2_extras_name_the_field(#[case] extras: &str, #[case] field: &str) {
    let error = build(extras).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidParam(name) if name.as_str() == field),
        "unexpected error {error}"
    );
}
