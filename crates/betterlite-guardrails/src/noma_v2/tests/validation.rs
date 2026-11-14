use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_guardrails::noma_v2::config::{NomaV2Params, NomaV2Settings};
use betterlite_guardrails::noma_v2::error::NomaV2Error;
use secrecy::{ExposeSecret, SecretString};

use crate::support::{empty_env, lookup, params};

#[test]
fn validation_treats_whitespace_only_values_as_none() {
    let mut configured = params();
    configured.api_key = Some(SecretString::from(String::from("   ")));
    configured.application_id = Some(String::from(" \t "));
    configured.gateway_name = Some(String::from("  "));
    configured.api_base = Some(String::from("https://noma.example"));
    let resolved = NomaV2Settings::resolve(&configured, &empty_env()).unwrap();
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.application_id, None);
    assert_eq!(resolved.gateway_name, None);
}

#[test]
fn validation_falls_back_to_the_environment_for_whitespace_only_values() {
    let mut configured = params();
    configured.api_key = Some(SecretString::from(String::from("  ")));
    configured.application_id = Some(String::from(" "));
    configured.gateway_name = Some(String::new());
    let env = lookup(&[
        ("NOMA_API_KEY", " env-key "),
        ("NOMA_APPLICATION_ID", " env-app "),
        ("NOMA_GATEWAY_NAME", " env-gateway "),
    ]);
    let resolved = NomaV2Settings::resolve(&configured, &env).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(resolved.application_id.as_deref(), Some("env-app"));
    assert_eq!(resolved.gateway_name.as_deref(), Some("env-gateway"));
}

#[test]
fn validation_trims_an_explicit_value() {
    let mut configured = params();
    configured.api_key = Some(SecretString::from(String::from("  key-1  ")));
    configured.application_id = Some(String::from(" app-1 "));
    configured.gateway_name = Some(String::from(" gw-1 "));
    let resolved = NomaV2Settings::resolve(&configured, &empty_env()).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("key-1")
    );
    assert_eq!(resolved.application_id.as_deref(), Some("app-1"));
    assert_eq!(resolved.gateway_name.as_deref(), Some("gw-1"));
}

#[test]
fn validation_rejects_a_negative_timeout() {
    let mut configured = params();
    configured.timeout = Some(-1.0_f64);
    let error = NomaV2Settings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(error, NomaV2Error::Timeout { .. }));
}

#[test]
fn validation_accepts_a_zero_timeout() {
    let mut configured = params();
    configured.api_key = Some(SecretString::from(String::from("key-1")));
    configured.timeout = Some(0.0_f64);
    let resolved = NomaV2Settings::resolve(&configured, &empty_env()).unwrap();
    assert_eq!(resolved.timeout, Some(Duration::ZERO));
}

#[test]
fn validation_rejects_a_wrongly_typed_monitor_mode() {
    let parsed = decode_litellm_params(
        br#"{"guardrail": "noma_v2", "mode": "pre_call", "api_base": "https://noma.example", "monitor_mode": "true"}"#,
    )
    .unwrap();
    let error = NomaV2Params::from_litellm(&parsed, "noma-v2", true).unwrap_err();
    assert!(matches!(
        error,
        NomaV2Error::Param {
            field: "monitor_mode"
        }
    ));
}

#[test]
fn validation_rejects_a_wrongly_typed_application_id() {
    let parsed = decode_litellm_params(
        br#"{"guardrail": "noma_v2", "mode": "pre_call", "api_base": "https://noma.example", "application_id": 7}"#,
    )
    .unwrap();
    let error = NomaV2Params::from_litellm(&parsed, "noma-v2", true).unwrap_err();
    assert!(matches!(
        error,
        NomaV2Error::Param {
            field: "application_id"
        }
    ));
}
