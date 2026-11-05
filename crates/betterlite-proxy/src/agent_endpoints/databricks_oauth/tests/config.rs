#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::agent_endpoints::databricks_oauth::config::{
    AgentLitellmParams, parse_databricks_oauth_config,
};

#[rstest]
#[case::resolve_secret_environ(
    r#"{"databricks_oauth":{"client_id":"os.environ/ID","client_secret":"s3cret","workspace_url":"https://w.example"}}"#,
    "https://w.example/oidc/v1/token|env-id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::resolve_secret_literal(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"os.environ/SECRET","workspace_url":"https://w.example"}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::resolve_secret_unset_environ_is_missing(
    r#"{"databricks_oauth":{"client_id":"os.environ/NOPE","client_secret":"s3cret","workspace_url":"https://w.example"}}"#,
    "Databricks App OAuth config is missing required field(s): client_id"
)]
#[case::resolve_secret_non_string_is_missing(
    r#"{"databricks_oauth":{"client_id":5,"client_secret":"s3cret","workspace_url":"https://w.example"}}"#,
    "Databricks App OAuth config is missing required field(s): client_id"
)]
#[case::workspace_url_trailing_slashes(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example///"}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::workspace_url_serving_endpoints(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example/serving-endpoints"}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::workspace_url_whitespace_and_serving_endpoints_slash(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"  https://w.example/serving-endpoints/ \n"}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::workspace_url_path_prefix_kept(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example/base"}}"#,
    "https://w.example/base/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::cache_key_secret_rotation(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"other","workspace_url":"https://w.example"}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|d9298a10d1b07358"
)]
#[case::parse_valid_config_with_scope(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example","scope":"os.environ/SCOPE"}}"#,
    "https://w.example/oidc/v1/token|id|sql|1ec1c26b50d5d3c5"
)]
#[case::parse_scope_default_when_empty(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example","scope":""}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::parse_scope_default_when_not_text(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret","workspace_url":"https://w.example","scope":7}}"#,
    "https://w.example/oidc/v1/token|id|all-apis|1ec1c26b50d5d3c5"
)]
#[case::parse_missing_client_id(
    r#"{"databricks_oauth":{"client_secret":"s3cret","workspace_url":"https://w.example"}}"#,
    "Databricks App OAuth config is missing required field(s): client_id"
)]
#[case::parse_missing_client_secret(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"","workspace_url":"https://w.example"}}"#,
    "Databricks App OAuth config is missing required field(s): client_secret"
)]
#[case::parse_missing_workspace_url(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s3cret"}}"#,
    "Databricks App OAuth config is missing required field(s): workspace_url"
)]
#[case::parse_missing_all_fields_in_order(
    r#"{"databricks_oauth":{}}"#,
    "Databricks App OAuth config is missing required field(s): client_id, client_secret, workspace_url"
)]
#[case::parse_invalid_block_type(
    r#"{"databricks_oauth":"oops"}"#,
    "'databricks_oauth' must be a mapping of OAuth settings"
)]
#[case::parse_invalid_block_list(
    r#"{"databricks_oauth":[]}"#,
    "'databricks_oauth' must be a mapping of OAuth settings"
)]
#[case::parse_no_block(r#"{"model":"agent"}"#, "none")]
#[case::parse_null_block(r#"{"databricks_oauth":null}"#, "none")]
fn databricks_oauth_tests_behavior(#[case] params: &str, #[case] expected: &str) {
    let environment = BTreeMap::from([
        ("ID".to_owned(), "env-id".to_owned()),
        ("SECRET".to_owned(), "s3cret".to_owned()),
        ("SCOPE".to_owned(), "sql".to_owned()),
    ]);
    let litellm_params: AgentLitellmParams = serde_json::from_str(params).unwrap();
    let observed = match parse_databricks_oauth_config(&litellm_params, &environment) {
        Ok(Some(config)) => config.cache_key(),
        Ok(None) => "none".to_owned(),
        Err(error) => error.to_string(),
    };
    assert_eq!(observed, expected);
}
