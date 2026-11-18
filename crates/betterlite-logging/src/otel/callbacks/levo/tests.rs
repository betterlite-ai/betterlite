#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{HEALTHY_MESSAGE, LevoHealth, health_check};
use crate::otel::integrations::arize_settings::OtlpProtocol;
use crate::otel::integrations::levo_settings::LevoConfig;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn complete() -> BTreeMap<String, String> {
    environment(&[
        ("LEVOAI_API_KEY", "levo-key"),
        ("LEVOAI_ORG_ID", "org-1"),
        ("LEVOAI_WORKSPACE_ID", "workspace-1"),
        (
            "LEVOAI_COLLECTOR_URL",
            "https://collector.levo.ai/v1/traces",
        ),
    ])
}

#[test]
fn levo_health_reports_healthy_when_all_four_variables_are_set() {
    let health = health_check(&complete());
    assert_eq!(
        health,
        LevoHealth::Healthy {
            message: HEALTHY_MESSAGE
        }
    );
    assert_eq!(
        serde_json::to_string(&health).expect("the health answer serializes"),
        r#"{"status":"healthy","message":"Levo credentials are configured properly"}"#
    );
}

#[rstest]
#[case::api_key(
    "LEVOAI_API_KEY",
    "LEVOAI_API_KEY environment variable is required for Levo integration."
)]
#[case::org_id(
    "LEVOAI_ORG_ID",
    "LEVOAI_ORG_ID environment variable is required for Levo integration."
)]
#[case::workspace_id(
    "LEVOAI_WORKSPACE_ID",
    "LEVOAI_WORKSPACE_ID environment variable is required for Levo integration."
)]
#[case::collector_url(
    "LEVOAI_COLLECTOR_URL",
    "LEVOAI_COLLECTOR_URL environment variable is required for Levo integration. Please contact Levo support to get your collector URL."
)]
fn levo_health_reports_a_missing_variable(#[case] missing: &str, #[case] message: &str) {
    let mut env = complete();
    env.remove(missing);
    let health = health_check(&env);
    assert_eq!(
        health,
        LevoHealth::Unhealthy {
            error_message: message.to_owned()
        }
    );
    assert_eq!(
        serde_json::to_string(&health).expect("the health answer serializes"),
        format!(
            r#"{{"status":"unhealthy","error_message":{}}}"#,
            quoted(message)
        )
    );
}

#[rstest]
#[case::api_key("LEVOAI_API_KEY")]
#[case::org_id("LEVOAI_ORG_ID")]
#[case::workspace_id("LEVOAI_WORKSPACE_ID")]
#[case::collector_url("LEVOAI_COLLECTOR_URL")]
fn levo_health_reports_an_empty_variable_as_missing(#[case] empty: &str) {
    let mut env = complete();
    env.insert(empty.to_owned(), String::new());
    assert!(matches!(health_check(&env), LevoHealth::Unhealthy { .. }));
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

#[test]
fn levo_otlp_auth_headers_join_authorization_organization_and_workspace() {
    let config =
        LevoConfig::from_env(&complete()).expect("the complete environment configures Levo");
    assert_eq!(
        config.otlp_auth_headers.expose_secret(),
        "Authorization=Bearer levo-key,x-levo-organization-id=org-1,x-levo-workspace-id=workspace-1"
    );
    assert_eq!(config.protocol, OtlpProtocol::Http);
}

#[test]
fn levo_collector_url_is_used_verbatim() {
    let mut env = complete();
    env.insert(
        "LEVOAI_COLLECTOR_URL".to_owned(),
        "https://collector.example/custom/path/".to_owned(),
    );
    let config = LevoConfig::from_env(&env).expect("the complete environment configures Levo");
    assert_eq!(config.endpoint, "https://collector.example/custom/path/");
    assert_eq!(config.protocol.exporter_kind(), "otlp_http");
}

#[test]
fn levo_health_marks_a_partial_configuration_unhealthy() {
    let env = environment(&[
        ("LEVOAI_API_KEY", "levo-key"),
        ("LEVOAI_WORKSPACE_ID", "workspace-1"),
        (
            "LEVOAI_COLLECTOR_URL",
            "https://collector.levo.ai/v1/traces",
        ),
    ]);
    let health = health_check(&env);
    assert_eq!(
        health,
        LevoHealth::Unhealthy {
            error_message: "LEVOAI_ORG_ID environment variable is required for Levo integration."
                .to_owned()
        }
    );
}
