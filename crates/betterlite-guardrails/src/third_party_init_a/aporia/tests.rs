#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use super::initialize_aporia;
use crate::third_party_init_a::tests::{exposed, params, params_in_mode};

#[test]
fn aporia_takes_the_defaults_of_an_empty_config() {
    let built = initialize_aporia(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn aporia_copies_the_configured_fields() {
    let built = initialize_aporia(
        &params_in_mode(
            r#""post_call""#,
            r#",
            "api_base": "https://aporia.test",
            "api_key": "aporia-key",
            "timeout": 12,
            "default_on": true
            "#,
        ),
        Some("aporia-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "aporia-guard");
    assert_eq!(built.api_base.as_deref(), Some("https://aporia.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("aporia-key"));
    assert_eq!(built.timeout, Some(12.0_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}
