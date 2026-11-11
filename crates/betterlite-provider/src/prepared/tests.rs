#![cfg(test)]

use std::time::Duration;

use betterlite_models::controls::call_params::DropParams;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
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

fn defaults_to_the_openai_base_and_long_timeout(result: &Result<Deployment, DeploymentError>) {
    let deployment = result.as_ref().expect("valid");
    assert_eq!(deployment.api_base, "https://api.openai.com/v1");
    assert_eq!(
        deployment.endpoint.as_str(),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(deployment.timeout, Duration::from_mins(100));
    assert!(deployment.api_key.is_none());
    assert!(deployment.organization.is_none());
    assert!(deployment.max_retries.is_none());
    assert!(deployment.drop_params.is_none());
}

fn reads_the_given_connection_values(result: &Result<Deployment, DeploymentError>) {
    let deployment = result.as_ref().expect("valid");
    assert_eq!(
        deployment.endpoint.as_str(),
        "http://h:1/base/chat/completions"
    );
    assert_eq!(deployment.api_key.as_ref().unwrap().expose_secret(), "k");
    assert_eq!(deployment.organization.as_deref(), Some("o"));
    assert_eq!(deployment.timeout, Duration::from_secs(12));
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
#[case::default_connection(params(&[("model", text("m"))]), defaults_to_the_openai_base_and_long_timeout)]
#[case::given_connection(params(&[("model", text("m")), ("api_base", text("http://h:1/base/")), ("api_key", text("k")), ("organization", text("o")), ("timeout", number(12))]), reads_the_given_connection_values)]
#[case::missing_model(params(&[]), rejects_missing_model)]
#[case::bad_api_base(params(&[("model", text("m")), ("api_base", text("not a url"))]), rejects_bad_api_base)]
#[case::bad_timeout(params(&[("model", text("m")), ("timeout", text("60"))]), rejects_bad_timeout)]
#[case::negative_timeout(params(&[("model", text("m")), ("timeout", number(-1))]), rejects_negative_timeout)]
#[case::string_flag_drop_params(params(&[("model", text("m")), ("drop_params", text("true"))]), keeps_flag_drop_params)]
fn provider_framework_tests_behavior(
    #[case] given: IndexMap<String, JsonValue>,
    #[case] check: fn(&Result<Deployment, DeploymentError>),
) {
    check(&Deployment::from_params(given));
}

mod identity;
