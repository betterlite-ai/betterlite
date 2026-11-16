use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use secrecy::ExposeSecret as _;

use crate::support::{empty_env, initialize, lookup, params};

#[test]
fn initialization_registers_the_guardrail_callback() {
    let env = lookup(&[("ARGUS_API_KEY", "env-key")]);
    let (result, registry) = initialize(&params(), &env);
    let guardrail = result.unwrap();
    assert_eq!(registry.callbacks.len(), 1);
    let registered: Arc<_> = Arc::clone(&registry.callbacks[0]);
    assert!(Arc::ptr_eq(&registered, &guardrail));
}

#[test]
fn initialization_reads_every_litellm_parameter() {
    let configured = crate::support::decoded_params(
        r#"{"guardrail": "repello", "mode": "post_call", "asset_id": "asset-9", "api_key": "param-key", "api_base": "http://example.test", "unreachable_fallback": "fail_open", "default_on": true, "timeout": 5.0}"#,
    );
    let (result, _registry) = initialize(&configured, &empty_env());
    let guardrail = result.unwrap();
    assert_eq!(guardrail.settings.api_base, "http://example.test");
    assert_eq!(guardrail.settings.asset_id, "asset-9");
    assert_eq!(guardrail.settings.api_key.expose_secret(), "param-key");
    assert_eq!(
        guardrail.settings.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
    assert_eq!(guardrail.settings.request_timeout, Duration::from_secs(5));
    assert_eq!(
        guardrail.base.settings.guardrail_name.as_deref(),
        Some("repello-test")
    );
    assert!(guardrail.base.settings.default_on);
    assert_eq!(
        guardrail.base.settings.event_hook,
        Some(LitellmParamsMode::One(String::from("post_call")))
    );
}

#[test]
fn initialization_reports_a_missing_asset_id() {
    let mut configured = params();
    configured.api_key = Some(secrecy::SecretString::from(String::from("explicit")));
    configured.asset_id = None;
    let (result, registry) = initialize(&configured, &empty_env());
    let Err(error) = result else {
        panic!("an absent asset id must be rejected");
    };
    assert_eq!(
        error,
        "Repello guardrail requires an `asset_id`. Create an asset in the Repello dashboard and set `asset_id` on the guardrail in the config file."
    );
    assert!(registry.callbacks.is_empty());
}

#[test]
fn an_invalid_event_hook_is_reported_before_the_missing_key() {
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(String::from("nope"));
    let (result, registry) = initialize(&configured, &empty_env());
    let Err(error) = result else {
        panic!("an invalid event hook must be rejected");
    };
    assert_eq!(
        error,
        "the RepelloAI Argus event hook `nope` is not a guardrail event hook"
    );
    assert!(registry.callbacks.is_empty());
}

#[test]
fn a_non_string_asset_id_is_a_configuration_error() {
    let body = r#"{"guardrail": "repello", "mode": "pre_call", "asset_id": 5}"#;
    let decoded = betterlite_config::sections::guardrail_params::decode::decode_litellm_params(
        body.as_bytes(),
    )
    .unwrap();
    let error = betterlite_guardrails::repello_lifecycle::config::RepelloAIParams::from_litellm(
        &decoded,
        "repello-test",
        true,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        betterlite_guardrails::repello_lifecycle::error::RepelloLifecycleError::InvalidParam {
            field: "asset_id"
        }
    ));
}
