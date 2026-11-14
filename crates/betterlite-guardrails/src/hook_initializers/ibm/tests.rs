#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_ibm;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

const REQUIRED: &str = r#""guardrail": "ibm_guardrails", "mode": "pre_call",
    "auth_token": "token", "base_url": "https://ibm.example", "detector_id": "jailbreak""#;

#[test]
fn ibm_applies_the_defaults_when_optional_params_is_absent() {
    let args = initialize_ibm(&params(&format!("{{{REQUIRED}}}")), Some("ibm-check")).unwrap();
    assert_eq!(args.guardrail_name, "ibm-check");
    assert_eq!(args.auth_token.expose_secret(), "token");
    assert_eq!(args.base_url, "https://ibm.example");
    assert_eq!(args.detector_id, "jailbreak");
    assert!(args.is_detector_server);
    assert!(args.detector_params.is_empty());
    assert!(args.extra_headers.is_empty());
    assert_eq!(args.score_threshold, None);
    assert!(args.block_on_detection);
    assert!(args.verify_ssl);
    assert!(!args.default_on);
    assert_eq!(args.timeout, None);
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn ibm_reads_optional_params_when_it_is_set() {
    let args = initialize_ibm(
        &params(&format!(
            r#"{{{REQUIRED}, "is_detector_server": false, "verify_ssl": false,
                "timeout": 4.5, "optional_params": {{
                    "detector_params": {{"threshold": 0.9}},
                    "extra_headers": {{"x-org": "acme"}},
                    "score_threshold": 0.4,
                    "block_on_detection": false}}}}"#
        )),
        Some("ibm-check"),
    )
    .unwrap();
    assert!(!args.is_detector_server);
    assert!(!args.verify_ssl);
    assert!(!args.block_on_detection);
    assert_eq!(args.score_threshold, Some(0.4_f64));
    assert_eq!(
        args.detector_params,
        BTreeMap::from([("threshold".to_owned(), GuardrailValue::Decimal(0.9))])
    );
    assert_eq!(
        args.extra_headers,
        BTreeMap::from([("x-org".to_owned(), GuardrailValue::Text("acme".to_owned()))])
    );
    assert_eq!(args.timeout, Some(4.5_f64));
}

#[test]
fn ibm_treats_null_optional_entries_as_absent() {
    let args = initialize_ibm(
        &params(&format!(
            r#"{{{REQUIRED}, "optional_params": {{"detector_params": null,
                "extra_headers": null, "score_threshold": null,
                "block_on_detection": null}}}}"#
        )),
        Some("ibm-check"),
    )
    .unwrap();
    assert!(args.detector_params.is_empty());
    assert!(args.extra_headers.is_empty());
    assert_eq!(args.score_threshold, None);
    assert!(args.block_on_detection);
    let null_optional = initialize_ibm(
        &params(&format!(r#"{{{REQUIRED}, "optional_params": null}}"#)),
        Some("ibm-check"),
    )
    .unwrap();
    assert!(null_optional.block_on_detection);
    assert!(null_optional.detector_params.is_empty());
}

#[rstest]
#[case::no_token(
    r#"{"guardrail": "ibm_guardrails", "mode": "pre_call"}"#,
    Some("ibm-check"),
    "IBM Guardrails: auth_token is required"
)]
#[case::empty_token(
    r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": ""}"#,
    Some("ibm-check"),
    "IBM Guardrails: auth_token is required"
)]
#[case::no_url(
    r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": "token"}"#,
    Some("ibm-check"),
    "IBM Guardrails: base_url is required"
)]
#[case::no_detector(
    r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": "token", "base_url": "https://ibm.example"}"#,
    Some("ibm-check"),
    "IBM Guardrails: detector_id is required"
)]
#[case::credentials_before_name(
    r#"{"guardrail": "ibm_guardrails", "mode": "pre_call"}"#,
    None,
    "IBM Guardrails: auth_token is required"
)]
fn ibm_refuses_missing_values_in_litellm_order(
    #[case] json: &str,
    #[case] name: Option<&str>,
    #[case] message: &str,
) {
    let error = initialize_ibm(&params(json), name).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn ibm_refuses_an_empty_guardrail_name() {
    let complete = params(&format!("{{{REQUIRED}}}"));
    let unnamed = initialize_ibm(&complete, Some("")).unwrap_err();
    assert_eq!(
        unnamed.to_string(),
        "IBM Guardrails: guardrail_name is required"
    );
}

#[test]
fn ibm_refuses_a_wrongly_typed_param_with_its_name() {
    let error = initialize_ibm(
        &params(&format!(
            r#"{{{REQUIRED}, "optional_params": {{"score_threshold": "high"}}}}"#
        )),
        Some("ibm-check"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "IBM Guardrails: score_threshold has an invalid value"
    );
    let wrong_token = initialize_ibm(
        &params(r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": 5}"#),
        Some("ibm-check"),
    )
    .unwrap_err();
    assert_eq!(
        wrong_token.to_string(),
        "IBM Guardrails: auth_token has an invalid value"
    );
}

#[test]
fn ibm_reads_integer_score_thresholds() {
    let signed = initialize_ibm(
        &params(&format!(
            r#"{{{REQUIRED}, "optional_params": {{"score_threshold": 1}}}}"#
        )),
        Some("ibm-check"),
    )
    .unwrap();
    assert_eq!(signed.score_threshold, Some(1.0_f64));
    let unsigned = initialize_ibm(
        &params(&format!(
            r#"{{{REQUIRED}, "optional_params": {{"score_threshold": 18446744073709551615}}}}"#
        )),
        Some("ibm-check"),
    )
    .unwrap();
    assert!(unsigned.score_threshold.is_some());
}
