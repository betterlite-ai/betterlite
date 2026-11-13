#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::initialize_lasso;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(extras: &str) -> super::LassoArgs {
    let body = format!(r#"{{"guardrail": "lasso", "mode": "post_call"{extras}}}"#);
    initialize_lasso(&params(&body), Some("lasso-check")).unwrap()
}

#[test]
fn lasso_args_copy_the_listed_fields() {
    let built = build(
        r#",
        "api_key": "lasso-key",
        "api_base": "https://lasso.example",
        "lasso_user_id": "user-1",
        "lasso_conversation_id": "conv-1",
        "mask": true,
        "default_on": true,
        "timeout": 4.0
        "#,
    );
    assert_eq!(built.name, "lasso-check");
    assert_eq!(
        built.lasso_api_key.as_ref().unwrap().expose_secret(),
        "lasso-key"
    );
    assert_eq!(built.api_base.as_deref(), Some("https://lasso.example"));
    assert_eq!(built.user_id.as_deref(), Some("user-1"));
    assert_eq!(built.conversation_id.as_deref(), Some("conv-1"));
    assert_eq!(built.mask, Some(true));
    assert!(built.default_on);
    assert_eq!(built.timeout, Some(4.0_f64));
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn lasso_args_take_the_declared_defaults() {
    let built = build("");
    assert_eq!(built.name, "lasso-check");
    assert!(built.lasso_api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.user_id, None);
    assert_eq!(built.conversation_id, None);
    assert_eq!(built.mask, Some(false));
    assert!(!built.default_on);
    assert_eq!(built.timeout, None);
}

#[test]
fn lasso_name_defaults_to_the_empty_string() {
    let built = initialize_lasso(
        &params(r#"{"guardrail": "lasso", "mode": "pre_call"}"#),
        None,
    )
    .unwrap();
    assert_eq!(built.name, "");
}
