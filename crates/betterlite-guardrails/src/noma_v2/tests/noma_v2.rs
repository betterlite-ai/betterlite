mod actions;
mod authorization;
mod config_model;
mod errors;
mod hooks;
mod monitor;
mod observability;
mod payload;
mod serialization;
mod validation;

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_guardrails::noma_v2::config::{NomaV2Params, NomaV2Settings};
use betterlite_guardrails::noma_v2::error::NomaV2Error;
use betterlite_guardrails::noma_v2::initialize_guardrail_v2;
use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret, SecretString};

use crate::support::{Registry, empty_env, lookup, params};

fn litellm(text: &str) -> LitellmParams {
    decode_litellm_params(text.as_bytes()).unwrap()
}

#[test]
fn config_from_environment_reads_every_variable() {
    let env = lookup(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_API_BASE", "https://noma.example/"),
        ("NOMA_APPLICATION_ID", "env-app"),
        ("NOMA_GATEWAY_NAME", "env-gateway"),
        ("NOMA_MONITOR_MODE", "true"),
        ("NOMA_BLOCK_FAILURES", "false"),
    ]);
    let settings = NomaV2Settings::resolve(&params(), &env).unwrap();
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(settings.api_base, "https://noma.example");
    assert_eq!(settings.application_id.as_deref(), Some("env-app"));
    assert_eq!(settings.gateway_name.as_deref(), Some("env-gateway"));
    assert!(settings.monitor_mode);
    assert!(!settings.block_failures);
    assert_eq!(settings.timeout, None);
}

#[test]
fn config_from_environment_defaults_api_base_and_flags() {
    let env = lookup(&[("NOMA_API_KEY", "env-key")]);
    let settings = NomaV2Settings::resolve(&params(), &env).unwrap();
    assert_eq!(settings.api_base, "https://api.noma.security");
    assert!(!settings.monitor_mode);
    assert!(settings.block_failures);
    assert_eq!(settings.application_id, None);
    assert_eq!(settings.gateway_name, None);
}

#[test]
fn config_from_environment_reads_an_empty_value_as_unset() {
    let env = lookup(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_API_BASE", ""),
        ("NOMA_APPLICATION_ID", ""),
        ("NOMA_GATEWAY_NAME", ""),
        ("NOMA_MONITOR_MODE", ""),
        ("NOMA_BLOCK_FAILURES", ""),
    ]);
    let settings = NomaV2Settings::resolve(&params(), &env).unwrap();
    assert_eq!(settings.api_base, "https://api.noma.security");
    assert!(!settings.monitor_mode);
    assert!(settings.block_failures);
    assert_eq!(settings.application_id, None);
    assert_eq!(settings.gateway_name, None);
}

#[test]
fn config_from_environment_rejects_an_invalid_boolean() {
    let env = lookup(&[("NOMA_API_KEY", "env-key"), ("NOMA_MONITOR_MODE", "maybe")]);
    let error = NomaV2Settings::resolve(&params(), &env).unwrap_err();
    assert!(matches!(error, NomaV2Error::Environment(_)));
}

#[test]
fn config_from_environment_reads_every_boolean_spelling() {
    let env = lookup(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_MONITOR_MODE", "yes"),
        ("NOMA_BLOCK_FAILURES", "off"),
    ]);
    let settings = NomaV2Settings::resolve(&params(), &env).unwrap();
    assert!(settings.monitor_mode);
    assert!(!settings.block_failures);
}

#[test]
fn config_constructor_overrides_environment() {
    let env = lookup(&[
        ("NOMA_API_KEY", "env-key"),
        ("NOMA_API_BASE", "https://env.example/"),
        ("NOMA_APPLICATION_ID", "env-app"),
        ("NOMA_GATEWAY_NAME", "env-gateway"),
        ("NOMA_MONITOR_MODE", "true"),
        ("NOMA_BLOCK_FAILURES", "false"),
    ]);
    let configured = NomaV2Params {
        guardrail_name: Some(String::from("noma-v2")),
        api_key: Some(SecretString::from(String::from("configured-key"))),
        api_base: Some(String::from("https://configured.example//")),
        application_id: Some(String::from("configured-app")),
        gateway_name: Some(String::from("configured-gateway")),
        monitor_mode: Some(false),
        block_failures: Some(true),
        timeout: Some(2.5),
        mode: Some(LitellmParamsMode::One(String::from("post_call"))),
        default_on: true,
        strict_modes: true,
    };
    let settings = NomaV2Settings::resolve(&configured, &env).unwrap();
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("configured-key")
    );
    assert_eq!(settings.api_base, "https://configured.example");
    assert_eq!(settings.application_id.as_deref(), Some("configured-app"));
    assert_eq!(settings.gateway_name.as_deref(), Some("configured-gateway"));
    assert!(!settings.monitor_mode);
    assert!(settings.block_failures);
    assert_eq!(
        settings.timeout,
        Some(Duration::try_from_secs_f64(2.5).unwrap())
    );
}

#[test]
fn initialize_guardrail_v2_registers_the_callback() {
    let mut registry = Registry::default();
    let env = lookup(&[("NOMA_API_KEY", "env-key")]);
    let guardrail = initialize_guardrail_v2(
        &params(),
        &env,
        base_builder().build().unwrap(),
        &mut registry,
    )
    .unwrap();
    assert_eq!(registry.callbacks.len(), 1);
    assert!(Arc::ptr_eq(&registry.callbacks[0], &guardrail));
    assert_eq!(guardrail.settings.api_base, "https://api.noma.security");
    assert_eq!(
        guardrail.base.settings.guardrail_name.as_deref(),
        Some("noma-v2-test")
    );
    assert_eq!(
        guardrail.base.settings.event_hook,
        Some(LitellmParamsMode::One(String::from("pre_call")))
    );
}

#[test]
fn initialize_guardrail_v2_carries_the_default_on_flag_and_timeout() {
    let mut configured = params();
    configured.mode = Some(LitellmParamsMode::One(String::from("post_call")));
    configured.default_on = true;
    configured.timeout = Some(0.5);
    let mut registry = Registry::default();
    let guardrail = initialize_guardrail_v2(
        &configured,
        &lookup(&[("NOMA_API_KEY", "env-key")]),
        base_builder().build().unwrap(),
        &mut registry,
    )
    .unwrap();
    assert!(guardrail.base.settings.default_on);
    assert_eq!(guardrail.base.settings.timeout, Some(0.5));
    assert_eq!(guardrail.settings.timeout, Some(Duration::from_millis(500)));
}

#[test]
fn initialize_guardrail_v2_requires_a_saas_api_key() {
    let mut registry = Registry::default();
    let error = initialize_guardrail_v2(
        &params(),
        &empty_env(),
        base_builder().build().unwrap(),
        &mut registry,
    )
    .unwrap_err();
    assert!(matches!(error, NomaV2Error::MissingApiKey));
    assert!(registry.callbacks.is_empty());
}

fn configured_params() -> NomaV2Params {
    let parsed = litellm(
        r#"{
            "guardrail": "noma_v2",
            "mode": "pre_call",
            "api_key": "configured-key",
            "api_base": "https://custom.example",
            "application_id": " app-1 ",
            "gateway_name": "gw",
            "monitor_mode": true,
            "block_failures": false,
            "default_on": true,
            "timeout": 4
        }"#,
    );
    NomaV2Params::from_litellm(&parsed, "noma-v2", true).unwrap()
}

#[test]
fn initialize_guardrail_v2_reads_litellm_params() {
    let from_params = configured_params();
    assert_eq!(from_params.guardrail_name.as_deref(), Some("noma-v2"));
    assert_eq!(
        from_params
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("configured-key")
    );
    assert_eq!(
        from_params.api_base.as_deref(),
        Some("https://custom.example")
    );
    assert_eq!(from_params.application_id.as_deref(), Some(" app-1 "));
    assert_eq!(
        from_params.mode,
        Some(LitellmParamsMode::One(String::from("pre_call")))
    );
    assert_eq!(from_params.timeout, Some(4.0));
    assert!(from_params.strict_modes);
}

#[test]
fn initialize_guardrail_v2_reads_every_flag() {
    let from_params = configured_params();
    assert_eq!(from_params.gateway_name.as_deref(), Some("gw"));
    assert_eq!(from_params.monitor_mode, Some(true));
    assert_eq!(from_params.block_failures, Some(false));
    assert!(from_params.default_on);
}

#[test]
fn initialize_guardrail_v2_resolves_the_configured_values() {
    let from_params = configured_params();
    let resolved = NomaV2Settings::resolve(&from_params, &empty_env()).unwrap();
    assert_eq!(resolved.application_id.as_deref(), Some("app-1"));
    assert_eq!(resolved.api_base, "https://custom.example");
}
