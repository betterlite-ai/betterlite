#![cfg(test)]

use betterlite_models::controls::call_params::DropParams;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::deployment::{Deployment, DeploymentError};

fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

fn number(value: i64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

fn params(pairs: &[(&str, JsonValue)]) -> IndexMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn parses_string_retries(result: &Result<Deployment, DeploymentError>) {
    assert_eq!(result.as_ref().expect("valid").max_retries, Some(3));
}

fn parses_number_retries(result: &Result<Deployment, DeploymentError>) {
    assert_eq!(result.as_ref().expect("valid").max_retries, Some(5));
}

fn rejects_bad_retries(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(result, Err(DeploymentError::MaxRetries { .. })));
}

fn rejects_bad_retries_type(result: &Result<Deployment, DeploymentError>) {
    let Err(DeploymentError::FieldType { field, expected }) = result else {
        panic!("a boolean max_retries is a field type error");
    };
    assert_eq!((*field, *expected), ("max_retries", "an integer"));
}

fn keeps_flag_drop_params(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(
        result.as_ref().expect("valid").drop_params,
        Some(DropParams::Flag(true))
    ));
}

fn parses_text_drop_params(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(
        &result.as_ref().expect("valid").drop_params,
        Some(DropParams::Text(given)) if given == "yes-please"
    ));
}

fn unsets_bad_drop_params(result: &Result<Deployment, DeploymentError>) {
    assert!(result.as_ref().expect("valid").drop_params.is_none());
}

fn keeps_affinity_header(result: &Result<Deployment, DeploymentError>) {
    assert_eq!(
        result
            .as_ref()
            .expect("valid")
            .provider_affinity_header
            .as_deref(),
        Some("x-session-id")
    );
}

fn accepts_params(result: &Result<Deployment, DeploymentError>) {
    assert_eq!(result.as_ref().expect("valid").model, "m");
}

fn rejects_forbidden_affinity(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(
        result,
        Err(DeploymentError::ProviderAffinityHeader { .. })
    ));
}

fn rejects_missing_model(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(result, Err(DeploymentError::MissingModel)));
}

fn rejects_bad_api_base(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(result, Err(DeploymentError::ApiBase { .. })));
}

fn rejects_bad_timeout(result: &Result<Deployment, DeploymentError>) {
    if let Err(DeploymentError::FieldType { field, .. }) = result {
        assert_eq!(*field, "timeout");
    } else {
        panic!("a string timeout is a field type error");
    }
}

fn rejects_negative_timeout(result: &Result<Deployment, DeploymentError>) {
    assert!(matches!(result, Err(DeploymentError::Timeout { .. })));
}

#[rstest]
#[case::string_max_retries(params(&[("model", text("m")), ("max_retries", text("3"))]), parses_string_retries)]
#[case::number_max_retries(params(&[("model", text("m")), ("max_retries", number(5))]), parses_number_retries)]
#[case::text_max_retries(params(&[("model", text("m")), ("max_retries", text("three"))]), rejects_bad_retries)]
#[case::bool_max_retries(params(&[("model", text("m")), ("max_retries", JsonValue::Bool(true))]), rejects_bad_retries_type)]
#[case::flag_drop_params(params(&[("model", text("m")), ("drop_params", JsonValue::Bool(true))]), keeps_flag_drop_params)]
#[case::text_drop_params(params(&[("model", text("m")), ("drop_params", text("yes-please"))]), parses_text_drop_params)]
#[case::number_drop_params(params(&[("model", text("m")), ("drop_params", number(5))]), unsets_bad_drop_params)]
#[case::affinity_header(params(&[("model", text("m")), ("provider_affinity_header", text("x-session-id"))]), keeps_affinity_header)]
#[case::forbidden_affinity(params(&[("model", text("m")), ("provider_affinity_header", text("authorization"))]), rejects_forbidden_affinity)]
#[case::reserved_keys(params(&[("self", text("x")), ("params", text("y")), ("__class__", text("z")), ("model", text("m"))]), accepts_params)]
#[case::missing_model(params(&[]), rejects_missing_model)]
#[case::bad_api_base(params(&[("model", text("m")), ("api_base", text("not a url"))]), rejects_bad_api_base)]
#[case::bad_timeout(params(&[("model", text("m")), ("timeout", text("60"))]), rejects_bad_timeout)]
#[case::negative_timeout(params(&[("model", text("m")), ("timeout", number(-1))]), rejects_negative_timeout)]
fn provider_framework_tests_behavior(
    #[case] given: IndexMap<String, JsonValue>,
    #[case] check: fn(&Result<Deployment, DeploymentError>),
) {
    check(&Deployment::from_params(given));
}

mod identity;
