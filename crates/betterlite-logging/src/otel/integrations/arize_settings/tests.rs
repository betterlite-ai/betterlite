#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::{ArizeConfig, OtlpProtocol};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::nothing_set(&[], OtlpProtocol::Grpc, "https://otlp.arize.com/v1")]
#[case::grpc_endpoint_wins(
    &[
        ("ARIZE_ENDPOINT", "grpc://collector:4317"),
        ("ARIZE_HTTP_ENDPOINT", "http://collector")
    ],
    OtlpProtocol::Grpc,
    "grpc://collector:4317"
)]
#[case::http_endpoint(
    &[("ARIZE_HTTP_ENDPOINT", "http://collector:6006")],
    OtlpProtocol::Http,
    "http://collector:6006"
)]
#[case::empty_endpoints_are_unset(
    &[("ARIZE_ENDPOINT", ""), ("ARIZE_HTTP_ENDPOINT", "")],
    OtlpProtocol::Grpc,
    "https://otlp.arize.com/v1"
)]
fn config_resolves_the_endpoint_and_protocol(
    #[case] variables: &[(&str, &str)],
    #[case] protocol: OtlpProtocol,
    #[case] endpoint: &str,
) {
    let config = ArizeConfig::from_env(&environment(variables));
    assert_eq!(config.protocol, protocol);
    assert_eq!(config.endpoint, endpoint);
}

#[test]
fn config_reads_the_credentials_and_project() {
    let config = ArizeConfig::from_env(&environment(&[
        ("ARIZE_SPACE_ID", "space"),
        ("ARIZE_SPACE_KEY", "space-key"),
        ("ARIZE_API_KEY", "api-key"),
        ("ARIZE_PROJECT_NAME", "project"),
    ]));
    assert_eq!(config.space_id.as_deref(), Some("space"));
    assert_eq!(config.space_key.as_deref(), Some("space-key"));
    assert_eq!(
        config.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("api-key")
    );
    assert_eq!(config.project_name.as_deref(), Some("project"));
}

#[test]
fn config_treats_empty_credentials_as_unset() {
    let config = ArizeConfig::from_env(&environment(&[
        ("ARIZE_SPACE_ID", ""),
        ("ARIZE_API_KEY", ""),
        ("ARIZE_PROJECT_NAME", ""),
    ]));
    assert_eq!(config.space_id, None);
    assert!(config.api_key.is_none());
    assert_eq!(config.project_name, None);
}

#[test]
fn config_debug_output_hides_the_api_key() {
    let config = ArizeConfig::from_env(&environment(&[("ARIZE_API_KEY", "sk-secret-value")]));
    assert!(!format!("{config:?}").contains("sk-secret-value"));
}

#[test]
fn exporter_kind_matches_the_protocol() {
    assert_eq!(OtlpProtocol::Grpc.exporter_kind(), "otlp_grpc");
    assert_eq!(OtlpProtocol::Http.exporter_kind(), "otlp_http");
}
