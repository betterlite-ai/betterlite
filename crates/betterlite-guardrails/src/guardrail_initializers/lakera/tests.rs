#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;
use secrecy::ExposeSecret as _;

use super::initialize_lakera;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

#[test]
fn lakera_args_copy_the_listed_fields() {
    let built = initialize_lakera(
        &params(
            r#"{
                "guardrail": "lakera",
                "mode": "during_call",
                "api_base": "https://lakera.example",
                "api_key": "lakera-key",
                "category_thresholds": {"prompt_injection": 0.5, "jailbreak": 0.7},
                "default_on": true,
                "timeout": 1.5
            }"#,
        ),
        Some("lakera-check"),
    );
    assert_eq!(built.name, "lakera-check");
    assert_eq!(built.api_base.as_deref(), Some("https://lakera.example"));
    assert_eq!(
        built.api_key.as_ref().unwrap().expose_secret(),
        "lakera-key"
    );
    assert_eq!(
        built.category_thresholds,
        Some(LakeraCategoryThresholds {
            prompt_injection: Some(0.5_f64),
            jailbreak: Some(0.7_f64),
        })
    );
    assert!(built.default_on);
    assert_eq!(built.timeout, Some(1.5_f64));
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("during_call".to_owned())
    );
}

#[test]
fn lakera_args_leave_absent_values_absent() {
    let built = initialize_lakera(
        &params(r#"{"guardrail": "lakera", "mode": "pre_call"}"#),
        None,
    );
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(built.category_thresholds, None);
    assert!(!built.default_on);
    assert_eq!(built.timeout, None);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}
