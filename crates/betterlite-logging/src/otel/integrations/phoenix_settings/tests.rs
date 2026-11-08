#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::PhoenixConfig;
use crate::otel::integrations::arize_settings::OtlpProtocol;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::no_collector(&[], OtlpProtocol::Http, "http://localhost:6006/v1/traces")]
#[case::grpc_scheme(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "grpc://collector:4317")],
    OtlpProtocol::Grpc,
    "grpc://collector:4317"
)]
#[case::port_4317(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector:4317")],
    OtlpProtocol::Grpc,
    "http://collector:4317"
)]
#[case::port_4317_with_traces_path(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector:4317/v1/traces")],
    OtlpProtocol::Http,
    "http://collector:4317/v1/traces"
)]
#[case::path_ends_with_v1(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector/v1")],
    OtlpProtocol::Http,
    "http://collector/v1/traces"
)]
#[case::path_ends_with_slash(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector/")],
    OtlpProtocol::Http,
    "http://collector/v1/traces"
)]
#[case::bare_host(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector")],
    OtlpProtocol::Http,
    "http://collector/v1/traces"
)]
#[case::traces_path_kept(
    &[("PHOENIX_COLLECTOR_ENDPOINT", "http://collector/v1/traces")],
    OtlpProtocol::Http,
    "http://collector/v1/traces"
)]
#[case::cloud_endpoint_kept(
    &[
        ("PHOENIX_API_KEY", "key"),
        ("PHOENIX_COLLECTOR_ENDPOINT", "https://app.phoenix.arize.com")
    ],
    OtlpProtocol::Http,
    "https://app.phoenix.arize.com"
)]
#[case::http_variable_wins(
    &[
        ("PHOENIX_COLLECTOR_HTTP_ENDPOINT", "http://http-collector"),
        ("PHOENIX_COLLECTOR_ENDPOINT", "http://plain-collector")
    ],
    OtlpProtocol::Http,
    "http://http-collector/v1/traces"
)]
#[case::empty_http_variable_is_unset(
    &[
        ("PHOENIX_COLLECTOR_HTTP_ENDPOINT", ""),
        ("PHOENIX_COLLECTOR_ENDPOINT", "http://collector")
    ],
    OtlpProtocol::Http,
    "http://collector/v1/traces"
)]
fn config_normalizes_the_collector_endpoint(
    #[case] variables: &[(&str, &str)],
    #[case] protocol: OtlpProtocol,
    #[case] endpoint: &str,
) {
    let config = PhoenixConfig::from_env(&environment(variables)).expect("the config resolves");
    assert_eq!(config.protocol, protocol);
    assert_eq!(config.endpoint, endpoint);
}

#[rstest]
#[case::http_casing(&[("PHOENIX_API_KEY", "key")], "Authorization=Bearer key")]
#[case::grpc_casing(
    &[
        ("PHOENIX_API_KEY", "key"),
        ("PHOENIX_COLLECTOR_ENDPOINT", "grpc://collector:4317")
    ],
    "authorization=Bearer key"
)]
#[case::empty_key_still_counts(&[("PHOENIX_API_KEY", "")], "Authorization=Bearer ")]
fn config_builds_the_auth_header(#[case] variables: &[(&str, &str)], #[case] expected: &str) {
    let config = PhoenixConfig::from_env(&environment(variables)).expect("the config resolves");
    assert_eq!(
        config
            .otlp_auth_headers
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(expected)
    );
}

#[test]
fn config_without_a_key_has_no_headers() {
    let config = PhoenixConfig::from_env(&environment(&[])).expect("the config resolves");
    assert!(config.otlp_auth_headers.is_none());
}

#[test]
fn config_debug_output_hides_the_api_key() {
    let config = PhoenixConfig::from_env(&environment(&[("PHOENIX_API_KEY", "sk-secret-value")]))
        .expect("the config resolves");
    let debug = format!("{config:?}");
    assert!(!debug.contains("sk-secret-value"));
    assert!(debug.contains("[REDACTED]"));
}

#[rstest]
#[case::cloud_http(&[("PHOENIX_COLLECTOR_ENDPOINT", "https://app.phoenix.arize.com")])]
#[case::cloud_grpc(&[("PHOENIX_COLLECTOR_ENDPOINT", "grpc://app.phoenix.arize.com:4317")])]
fn config_requires_a_key_for_phoenix_cloud(#[case] variables: &[(&str, &str)]) {
    let error =
        PhoenixConfig::from_env(&environment(variables)).expect_err("the cloud needs a key");
    assert_eq!(
        error.to_string(),
        "PHOENIX_API_KEY must be set when using Phoenix Cloud (app.phoenix.arize.com)."
    );
}

#[rstest]
#[case::default(&[], "default")]
#[case::named(&[("PHOENIX_PROJECT_NAME", "project")], "project")]
#[case::empty_name_is_unset(&[("PHOENIX_PROJECT_NAME", "")], "default")]
fn config_reads_the_project_name(#[case] variables: &[(&str, &str)], #[case] expected: &str) {
    let config = PhoenixConfig::from_env(&environment(variables)).expect("the config resolves");
    assert_eq!(config.project_name, expected);
}
