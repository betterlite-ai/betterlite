#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ThirdPartyInitAError;
use super::{config_value, decode, optional_params};

pub(super) fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "test", "mode": "pre_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

pub(super) fn params_in_mode(mode: &str, extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "test", "mode": {mode}{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

pub(super) fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(SecretString::expose_secret)
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(GuardrailValue::Null))]
fn decode_reads_an_absent_or_null_value_as_none(#[case] value: Option<GuardrailValue>) {
    let decoded: Option<String> = decode("field", value.as_ref()).unwrap();
    assert_eq!(decoded, None);
}

#[test]
fn decode_reads_a_value_of_the_declared_type() {
    let value = GuardrailValue::Text("abc".to_owned());
    let decoded: Option<String> = decode("field", Some(&value)).unwrap();
    assert_eq!(decoded.as_deref(), Some("abc"));
}

#[test]
fn decode_names_the_field_of_a_wrong_type() {
    let value = GuardrailValue::Boolean(true);
    let error = decode::<String>("field", Some(&value)).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == "field"
    ));
    assert_eq!(
        error.to_string(),
        "the guardrail parameter field has an invalid value"
    );
}

#[test]
fn optional_params_is_none_when_absent_or_null() {
    assert!(optional_params(&params("").base.extra).unwrap().is_none());
    assert!(
        optional_params(&params(r#", "optional_params": null"#).base.extra)
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::text(r#", "optional_params": "text""#)]
#[case::list(r#", "optional_params": [1]"#)]
fn optional_params_refuses_a_value_that_is_not_an_object(#[case] extras: &str) {
    let error = optional_params(&params(extras).base.extra).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == "optional_params"
    ));
}

#[test]
fn config_value_prefers_a_non_null_optional_entry() {
    let built = params(r#", "k": "top", "optional_params": {"k": "nested"}"#);
    let optional = optional_params(&built.base.extra).unwrap();
    let value = config_value(optional, &built.base.extra.0, "k");
    assert_eq!(value, Some(&GuardrailValue::Text("nested".to_owned())));
}

#[rstest]
#[case::null_entry(r#", "k": "top", "optional_params": {"k": null}"#, Some("top"))]
#[case::missing_entry(r#", "k": "top", "optional_params": {"other": 1}"#, Some("top"))]
#[case::no_optional_params(r#", "k": "top""#, Some("top"))]
#[case::nothing_anywhere(r#", "optional_params": {"k": null}"#, None)]
fn config_value_falls_back_to_the_top_level(#[case] extras: &str, #[case] expected: Option<&str>) {
    let built = params(extras);
    let optional = optional_params(&built.base.extra).unwrap();
    let value = config_value(optional, &built.base.extra.0, "k");
    let want = expected.map(|text| GuardrailValue::Text(text.to_owned()));
    assert_eq!(value, want.as_ref());
}
