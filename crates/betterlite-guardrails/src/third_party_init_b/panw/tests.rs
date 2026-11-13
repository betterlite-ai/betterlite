#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::{SetParam, initialize_panw_prisma_airs};
use crate::third_party_init_b::error::ThirdPartyInitBError;

fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "panw_prisma_airs", "mode": "pre_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn panw_refuses_a_missing_or_empty_name() {
    let missing = initialize_panw_prisma_airs(&params(""), None).unwrap_err();
    assert!(matches!(missing, ThirdPartyInitBError::MissingPanwName));
    assert_eq!(
        missing.to_string(),
        "PANW Prisma AIRS: guardrail_name is required"
    );
    let empty = initialize_panw_prisma_airs(&params(""), Some("")).unwrap_err();
    assert!(matches!(empty, ThirdPartyInitBError::MissingPanwName));
}

#[test]
fn panw_args_copy_the_name_event_hook_and_default_on() {
    let built = initialize_panw_prisma_airs(&params(r#", "default_on": true"#), Some("panw-check"))
        .unwrap();
    assert_eq!(built.name, "panw-check");
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(built.default_on);
}

#[test]
fn panw_set_params_hold_only_the_values_the_config_wrote() {
    let built = initialize_panw_prisma_airs(
        &params(r#", "api_key": "panw-key", "profile_name": "profile""#),
        Some("panw-check"),
    )
    .unwrap();
    assert!(matches!(
        built.set_params.get("profile_name"),
        Some(SetParam::Value(GuardrailValue::Text(text))) if text == "profile"
    ));
    assert!(matches!(
        built.set_params.get("api_key"),
        Some(SetParam::Secret(_))
    ));
    assert!(!built.set_params.contains_key("mask_on_block"));
    assert!(!built.set_params.contains_key("app_name"));
    assert!(!built.set_params.contains_key("api_base"));
    assert!(!built.set_params.contains_key("timeout"));
}

#[test]
fn panw_set_params_hold_default_on_even_when_the_config_left_it_out() {
    let built = initialize_panw_prisma_airs(&params(""), Some("panw-check")).unwrap();
    assert!(matches!(
        built.set_params.get("default_on"),
        Some(SetParam::Value(GuardrailValue::Boolean(false)))
    ));
}

#[test]
fn panw_set_params_keep_an_explicit_null() {
    let built =
        initialize_panw_prisma_airs(&params(r#", "api_base": null"#), Some("panw-check")).unwrap();
    assert!(matches!(
        built.set_params.get("api_base"),
        Some(SetParam::Value(GuardrailValue::Null))
    ));
}

#[test]
fn panw_set_params_keep_the_secret_out_of_debug() {
    let built = initialize_panw_prisma_airs(
        &params(r#", "api_key": "sk-hidden-value""#),
        Some("panw-check"),
    )
    .unwrap();
    let Some(SetParam::Secret(secret)) = built.set_params.get("api_key") else {
        panic!("the api key is not a secret");
    };
    assert_eq!(secret.expose_secret(), "sk-hidden-value");
    assert!(!format!("{built:?}").contains("sk-hidden-value"));
}

#[test]
fn panw_set_params_hold_unknown_extras() {
    let built =
        initialize_panw_prisma_airs(&params(r#", "mask_on_block": true"#), Some("panw-check"))
            .unwrap();
    assert!(matches!(
        built.set_params.get("mask_on_block"),
        Some(SetParam::Value(GuardrailValue::Boolean(true)))
    ));
}
