#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{WeaveOtelConfig, weave_authorization_header, weave_otel_endpoint};
use crate::otel::integrations::arize_settings::OtlpProtocol;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::cloud_default(None, "https://trace.wandb.ai/otel/v1/traces")]
#[case::empty_host_is_unset(Some(""), "https://trace.wandb.ai/otel/v1/traces")]
#[case::host_without_scheme(Some("wandb.example"), "https://wandb.example/otel/v1/traces")]
#[case::host_with_scheme(Some("https://wandb.example"), "https://wandb.example/otel/v1/traces")]
#[case::trailing_slashes_dropped(
    Some("https://wandb.example///"),
    "https://wandb.example/otel/v1/traces"
)]
#[case::http_prefix_is_kept_as_given(Some("httpbin.example"), "httpbin.example/otel/v1/traces")]
fn endpoint_appends_the_otel_path(#[case] host: Option<&str>, #[case] expected: &str) {
    assert_eq!(weave_otel_endpoint(host), expected);
}

#[test]
fn authorization_header_is_basic_api_key() {
    assert_eq!(weave_authorization_header("abc"), "Basic YXBpOmFiYw==");
}

#[test]
fn config_builds_the_cloud_request() {
    let config = WeaveOtelConfig::from_env(&environment(&[
        ("WANDB_API_KEY", "abc"),
        ("WANDB_PROJECT_ID", "entity/project"),
    ]))
    .expect("the config resolves");
    assert_eq!(config.protocol, OtlpProtocol::Http);
    assert_eq!(config.endpoint, "https://trace.wandb.ai/otel/v1/traces");
    assert_eq!(config.project_id, "entity/project");
    assert_eq!(
        config.otlp_auth_headers.expose_secret(),
        "Authorization=Basic YXBpOmFiYw==,project_id=entity/project"
    );
}

#[test]
fn config_uses_the_configured_host() {
    let config = WeaveOtelConfig::from_env(&environment(&[
        ("WANDB_API_KEY", "abc"),
        ("WANDB_PROJECT_ID", "entity/project"),
        ("WANDB_HOST", "https://wandb.example/"),
    ]))
    .expect("the config resolves");
    assert_eq!(config.endpoint, "https://wandb.example/otel/v1/traces");
}

#[test]
fn config_debug_output_hides_the_api_key() {
    let config = WeaveOtelConfig::from_env(&environment(&[
        ("WANDB_API_KEY", "sk-secret-value"),
        ("WANDB_PROJECT_ID", "entity/project"),
    ]))
    .expect("the config resolves");
    let debug = format!("{config:?}");
    assert!(!debug.contains("sk-secret-value"));
    assert!(debug.contains("[REDACTED]"));
}

#[rstest]
#[case::api_key(&[], "WANDB_API_KEY must be set for Weave OpenTelemetry integration.")]
#[case::project_id(
    &[("WANDB_API_KEY", "abc")],
    "WANDB_PROJECT_ID must be set for Weave OpenTelemetry integration. Format: <entity>/<project_name>"
)]
#[case::empty_api_key(
    &[("WANDB_API_KEY", ""), ("WANDB_PROJECT_ID", "entity/project")],
    "WANDB_API_KEY must be set for Weave OpenTelemetry integration."
)]
fn config_requires_the_credentials(#[case] variables: &[(&str, &str)], #[case] message: &str) {
    let error = WeaveOtelConfig::from_env(&environment(variables)).expect_err("a value is missing");
    assert_eq!(error.to_string(), message);
}
