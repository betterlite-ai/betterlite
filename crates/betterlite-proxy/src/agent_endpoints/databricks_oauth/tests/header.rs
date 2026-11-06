#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{client, upstream};
use crate::agent_endpoints::databricks_oauth::config::AgentLitellmParams;
use crate::agent_endpoints::databricks_oauth::header::resolve_oauth_header;
use crate::agent_endpoints::databricks_oauth::token_cache::DatabricksAppOAuthTokenCache;

// WORKSPACE stands for the local upstream's URL.
#[rstest]
#[case::resolver_entry_point_without_block(r#"{"model":"agent"}"#, 1, "none", 0)]
#[case::resolver_entry_point_with_null_block(r#"{"databricks_oauth":null}"#, 1, "none", 0)]
#[case::resolver_entry_point_bearer_token(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s","workspace_url":"WORKSPACE"}}"#,
    1,
    "Bearer tok",
    1
)]
#[case::resolver_entry_point_caches_across_calls(
    r#"{"databricks_oauth":{"client_id":"id","client_secret":"s","workspace_url":"WORKSPACE/serving-endpoints"}}"#,
    3,
    "Bearer tok",
    1
)]
#[case::resolver_entry_point_rejects_incomplete_block(
    r#"{"databricks_oauth":{"client_id":"id","workspace_url":"WORKSPACE"}}"#,
    1,
    "Databricks App OAuth config is missing required field(s): client_secret",
    0
)]
#[tokio::test]
async fn databricks_oauth_tests_behavior(
    #[case] params: &str,
    #[case] calls: usize,
    #[case] expected: &str,
    #[case] expected_requests: usize,
) {
    let server = upstream(
        200,
        r#"{"access_token":"tok","expires_in":3600}"#,
        Duration::ZERO,
    )
    .await;
    let litellm_params: AgentLitellmParams =
        serde_json::from_str(&params.replace("WORKSPACE", &server.url)).unwrap();
    let cache = DatabricksAppOAuthTokenCache::new(client(), Arc::new(|| 0.0_f64)).unwrap();
    let observed = call_repeatedly(&cache, &litellm_params, calls).await;
    assert_eq!(observed, expected);
    assert_eq!(server.requests.lock().unwrap().len(), expected_requests);
}

async fn call_repeatedly(
    cache: &DatabricksAppOAuthTokenCache,
    params: &AgentLitellmParams,
    calls: usize,
) -> String {
    let mut observed = String::new();
    for _ in 0..calls {
        observed = match resolve_oauth_header(cache, params, &BTreeMap::new()).await {
            Ok(Some(header)) => header.expose_secret().to_owned(),
            Ok(None) => "none".to_owned(),
            Err(error) => error.to_string(),
        };
    }
    observed
}
