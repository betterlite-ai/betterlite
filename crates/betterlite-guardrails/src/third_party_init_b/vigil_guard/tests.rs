#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::initialize_vigil_guard;

fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "vigil_guard", "mode": "post_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn vigil_guard_copies_the_config_values() {
    let built = initialize_vigil_guard(
        &params(
            r#",
            "api_key": "vigil-key",
            "api_base": "https://vigil.test",
            "unreachable_fallback": "fail_open",
            "timeout": 7.5,
            "default_on": true
            "#,
        ),
        Some("vigil-check"),
    )
    .unwrap();
    assert_eq!(built.name, "vigil-check");
    assert_eq!(
        built
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("vigil-key".to_owned())
    );
    assert_eq!(built.api_base.as_deref(), Some("https://vigil.test"));
    assert_eq!(built.unreachable_fallback, "fail_open");
    assert_eq!(built.timeout, Some(7.5_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn vigil_guard_applies_the_defaults() {
    let built = initialize_vigil_guard(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.unreachable_fallback, "fail_closed");
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
}
