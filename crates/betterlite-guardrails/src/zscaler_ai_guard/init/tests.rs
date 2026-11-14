#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::{ZscalerInitError, initialize_zscaler};

fn params(extra: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "zscaler_ai_guard", "mode": "post_call"{extra}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn initialize_copies_the_listed_fields() {
    let args = initialize_zscaler(
        &params(
            r#",
            "api_base": "https://zscaler.example/policy",
            "api_key": "zscaler-key",
            "policy_id": 7,
            "send_user_api_key_alias": true,
            "send_user_api_key_user_id": null,
            "timeout": 2.5,
            "default_on": true
            "#,
        ),
        Some("zscaler-check"),
    )
    .unwrap();
    assert_eq!(args.name, "zscaler-check");
    assert_eq!(
        args.api_base.as_deref(),
        Some("https://zscaler.example/policy")
    );
    assert_eq!(
        args.api_key.as_ref().unwrap().expose_secret(),
        "zscaler-key"
    );
    assert_eq!(args.policy_id, Some(7));
    assert_eq!(args.send_user_api_key_alias, Some(true));
    assert_eq!(args.send_user_api_key_user_id, None);
    assert_eq!(args.send_user_api_key_team_id, Some(false));
    assert_eq!(args.timeout, Some(2.5_f64));
    assert!(args.default_on);
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn initialize_takes_the_declared_defaults() {
    let args = initialize_zscaler(&params(""), None).unwrap();
    assert_eq!(args.name, "");
    assert_eq!(args.api_base, None);
    assert!(args.api_key.is_none());
    assert_eq!(args.policy_id, None);
    assert_eq!(args.send_user_api_key_alias, Some(false));
    assert_eq!(args.send_user_api_key_user_id, Some(false));
    assert_eq!(args.send_user_api_key_team_id, Some(false));
    assert_eq!(args.timeout, None);
    assert!(!args.default_on);
}

#[test]
fn initialize_keeps_an_explicit_null_policy_id() {
    let args = initialize_zscaler(&params(r#", "policy_id": null"#), None).unwrap();
    assert_eq!(args.policy_id, None);
}

#[test]
fn initialize_refuses_a_non_integer_policy_id() {
    let error = initialize_zscaler(&params(r#", "policy_id": "seven""#), None).unwrap_err();
    assert!(matches!(error, ZscalerInitError::InvalidParam { name } if name == "policy_id"));
}

#[test]
fn initialize_refuses_a_non_boolean_send_flag() {
    let error =
        initialize_zscaler(&params(r#", "send_user_api_key_team_id": "yes""#), None).unwrap_err();
    assert!(matches!(
        error,
        ZscalerInitError::InvalidParam {
            name: "send_user_api_key_team_id"
        }
    ));
}
