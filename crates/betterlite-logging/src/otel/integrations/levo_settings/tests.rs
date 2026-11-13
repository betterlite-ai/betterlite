#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::LevoConfig;
use crate::otel::integrations::arize_settings::OtlpProtocol;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn config_builds_the_collector_request() {
    let config = LevoConfig::from_env(&environment(&[
        ("LEVOAI_API_KEY", "key"),
        ("LEVOAI_ORG_ID", "org"),
        ("LEVOAI_WORKSPACE_ID", "workspace"),
        ("LEVOAI_COLLECTOR_URL", "https://levo.example/v1/traces"),
    ]))
    .expect("the config resolves");
    assert_eq!(config.protocol, OtlpProtocol::Http);
    assert_eq!(config.endpoint, "https://levo.example/v1/traces");
    assert_eq!(
        config.otlp_auth_headers.expose_secret(),
        "Authorization=Bearer key,x-levo-organization-id=org,x-levo-workspace-id=workspace"
    );
}

#[test]
fn config_debug_output_hides_the_api_key() {
    let config = LevoConfig::from_env(&environment(&[
        ("LEVOAI_API_KEY", "sk-secret-value"),
        ("LEVOAI_ORG_ID", "org"),
        ("LEVOAI_WORKSPACE_ID", "workspace"),
        ("LEVOAI_COLLECTOR_URL", "https://levo.example/v1/traces"),
    ]))
    .expect("the config resolves");
    let debug = format!("{config:?}");
    assert!(!debug.contains("sk-secret-value"));
    assert!(debug.contains("[REDACTED]"));
}

#[rstest]
#[case::api_key(&[], "LEVOAI_API_KEY environment variable is required for Levo integration.")]
#[case::org_id(
    &[("LEVOAI_API_KEY", "key")],
    "LEVOAI_ORG_ID environment variable is required for Levo integration."
)]
#[case::workspace_id(
    &[("LEVOAI_API_KEY", "key"), ("LEVOAI_ORG_ID", "org")],
    "LEVOAI_WORKSPACE_ID environment variable is required for Levo integration."
)]
#[case::collector_url(
    &[
        ("LEVOAI_API_KEY", "key"),
        ("LEVOAI_ORG_ID", "org"),
        ("LEVOAI_WORKSPACE_ID", "workspace")
    ],
    "LEVOAI_COLLECTOR_URL environment variable is required for Levo integration. Please contact Levo support to get your collector URL."
)]
fn config_requires_each_variable(#[case] variables: &[(&str, &str)], #[case] message: &str) {
    let error = LevoConfig::from_env(&environment(variables)).expect_err("a variable is missing");
    assert_eq!(error.to_string(), message);
}

#[test]
fn config_reports_the_first_missing_variable_in_order() {
    let error = LevoConfig::from_env(&environment(&[
        ("LEVOAI_API_KEY", ""),
        ("LEVOAI_WORKSPACE_ID", "workspace"),
        ("LEVOAI_COLLECTOR_URL", "https://levo.example/v1/traces"),
    ]))
    .expect_err("the API key is missing");
    assert_eq!(
        error.to_string(),
        "LEVOAI_API_KEY environment variable is required for Levo integration."
    );
}
