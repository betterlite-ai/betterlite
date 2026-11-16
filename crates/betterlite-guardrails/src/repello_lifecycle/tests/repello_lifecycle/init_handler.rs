use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::repello_lifecycle::config::{
    RepelloAIParams, RepelloAISettings, RepelloLifecycleError,
};
use betterlite_guardrails::repello_lifecycle::guardrail_settings;
use rstest::rstest;

use crate::support::{GUARDRAIL_NAME, empty_env, lookup, params};

fn resolved(configured: &RepelloAIParams) -> RepelloAISettings {
    let env = lookup(&[("ARGUS_API_KEY", "env-key")]);
    RepelloAISettings::resolve(configured, &env).unwrap()
}

#[test]
fn the_default_request_timeout_is_thirty_seconds() {
    let settings = resolved(&params());
    assert_eq!(settings.request_timeout, Duration::from_secs(30));
}

#[test]
fn a_configured_timeout_is_the_request_timeout() {
    let mut configured = params();
    configured.timeout = Some(2.5);
    let settings = resolved(&configured);
    assert_eq!(settings.request_timeout, Duration::from_millis(2_500));
}

#[rstest]
#[case::negative(-1.0)]
#[case::not_a_number(f64::NAN)]
#[case::infinite(f64::INFINITY)]
fn a_timeout_that_is_not_a_duration_is_rejected(#[case] seconds: f64) {
    let mut configured = params();
    configured.timeout = Some(seconds);
    let error = RepelloAISettings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(error, RepelloLifecycleError::Timeout { .. }));
}

#[test]
fn the_parent_settings_chain_every_guardrail_field() {
    let configured = params();
    let settings = guardrail_settings(&configured, configured.mode.clone());
    assert_eq!(settings.guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One(String::from("pre_call")))
    );
    assert!(settings.default_on);
    assert_eq!(settings.timeout, None);
    assert!(settings.sticky_session_routing);
    assert!(!settings.run_in_parallel);
    assert!(!settings.scan_raw_request);
    assert!(settings.switches.records_own_information);
    assert!(!settings.switches.use_native_lifecycle_hooks);
    assert!(!settings.switches.use_native_during_call_hook);
    assert!(!settings.switches.uses_apply_guardrail_interface);
}

#[test]
fn the_parent_settings_carry_the_timeout_and_default_on() {
    let mut configured = params();
    configured.timeout = Some(7.0);
    configured.default_on = false;
    let settings = guardrail_settings(&configured, configured.mode.clone());
    assert_eq!(settings.timeout, Some(7.0));
    assert!(!settings.default_on);
}

#[test]
fn a_non_numeric_timeout_fails_to_decode() {
    let body =
        r#"{"guardrail": "repello", "mode": "pre_call", "asset_id": "asset-1", "timeout": "fast"}"#;
    assert!(decode_litellm_params(body.as_bytes()).is_err());
}
