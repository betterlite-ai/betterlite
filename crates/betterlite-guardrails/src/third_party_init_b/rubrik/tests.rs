#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::initialize_rubrik;

fn params(mode: &str, extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "rubrik", "mode": {mode}{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn rubrik_copies_the_config_values() {
    let built = initialize_rubrik(
        &params(
            r#""post_call""#,
            r#", "api_key": "rubrik-key", "api_base": "https://rubrik.test", "timeout": 4.5, "default_on": true"#,
        ),
        Some("rubrik-check"),
    )
    .unwrap();
    assert_eq!(built.name, "rubrik-check");
    assert_eq!(
        built
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("rubrik-key".to_owned())
    );
    assert_eq!(built.api_base.as_deref(), Some("https://rubrik.test"));
    assert_eq!(built.timeout, Some(4.5_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn rubrik_applies_the_defaults() {
    let built = initialize_rubrik(&params(r#""pre_call""#, ""), None).unwrap();
    assert_eq!(built.name, "");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn rubrik_keeps_a_mode_list_as_the_event_hook() {
    let built = initialize_rubrik(&params(r#"["pre_call", "post_call"]"#, ""), None).unwrap();
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
}
