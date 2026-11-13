#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::{QualifireArgs, QualifireInitError, initialize_qualifire};

fn params(extra: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "qualifire", "mode": "pre_call"{extra}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

fn build(extra: &str) -> Result<QualifireArgs, QualifireInitError> {
    initialize_qualifire(&params(extra), Some("qualifire-check"))
}

#[test]
fn qualifire_args_copy_the_connection_settings() {
    let built =
        build(r#", "api_base": "https://qualifire.test", "api_key": "key", "timeout": 3.5"#)
            .unwrap();
    assert_eq!(built.api_base.as_deref(), Some("https://qualifire.test"));
    assert_eq!(built.api_key.unwrap().expose_secret(), "key");
    assert_eq!(built.timeout, Some(3.5));
}

#[test]
fn qualifire_args_copy_every_check_flag() {
    let built = build(
        r#", "prompt_injections": true, "hallucinations_check": true, "grounding_check": true, "pii_check": true, "content_moderation_check": true, "tool_selection_quality_check": true"#,
    )
    .unwrap();
    assert_eq!(built.prompt_injections, Some(true));
    assert_eq!(built.hallucinations_check, Some(true));
    assert_eq!(built.grounding_check, Some(true));
    assert_eq!(built.pii_check, Some(true));
    assert_eq!(built.content_moderation_check, Some(true));
    assert_eq!(built.tool_selection_quality_check, Some(true));
}

#[test]
fn qualifire_args_copy_the_evaluation_settings() {
    let built = build(r#", "evaluation_id": "e1", "assertions": ["be brief"]"#).unwrap();
    assert_eq!(built.evaluation_id.as_deref(), Some("e1"));
    assert_eq!(built.assertions, Some(vec!["be brief".to_owned()]));
}

#[test]
fn qualifire_args_default_on_flagged_to_block() {
    let built = build("").unwrap();
    assert_eq!(built.on_flagged.as_deref(), Some("block"));
}

#[test]
fn qualifire_args_copy_a_given_on_flagged() {
    let built = build(r#", "on_flagged": "monitor""#).unwrap();
    assert_eq!(built.on_flagged.as_deref(), Some("monitor"));
}

#[test]
fn qualifire_args_leave_absent_settings_absent() {
    let built = build("").unwrap();
    assert_eq!(built.api_key, None);
    assert_eq!(built.api_base, None);
    assert_eq!(built.evaluation_id, None);
    assert_eq!(built.prompt_injections, None);
    assert_eq!(built.assertions, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
}

#[test]
fn qualifire_args_leave_null_settings_absent() {
    let built = build(
        r#", "evaluation_id": null, "prompt_injections": null, "assertions": null, "on_flagged": null, "timeout": null"#,
    )
    .unwrap();
    assert_eq!(built.evaluation_id, None);
    assert_eq!(built.prompt_injections, None);
    assert_eq!(built.assertions, None);
    assert_eq!(built.on_flagged, None);
    assert_eq!(built.timeout, None);
}

#[test]
fn qualifire_args_take_the_guardrail_name() {
    let built = build("").unwrap();
    assert_eq!(built.name, "qualifire-check");
    let unnamed = initialize_qualifire(&params(""), None).unwrap();
    assert_eq!(unnamed.name, "");
}

#[test]
fn qualifire_args_take_the_event_hook_and_default_on() {
    let body = r#"{"guardrail": "qualifire", "mode": "post_call", "default_on": true}"#;
    let decoded = decode_litellm_params(body.as_bytes()).unwrap();
    let built = initialize_qualifire(&decoded, Some("q")).unwrap();
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
    assert!(built.default_on);
}

#[test]
fn qualifire_args_reject_a_wrong_typed_flag() {
    let error = build(r#", "prompt_injections": "yes""#).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail parameter prompt_injections has an invalid value"
    );
}

#[test]
fn qualifire_args_reject_wrong_typed_assertions() {
    let error = build(r#", "assertions": "be nice""#).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail parameter assertions has an invalid value"
    );
}
