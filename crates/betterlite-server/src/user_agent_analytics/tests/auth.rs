use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use crate::user_agent_analytics::tests::support::DetailBody;
use crate::user_agent_analytics::tests::support::ErrorEnvelope;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::NestedBody;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::get;

const DB_DETAIL: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";
const QUERY_FAILURE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

#[rstest]
#[case::distinct("/tag/distinct")]
#[case::daily("/tag/dau")]
#[case::weekly("/tag/wau")]
#[tokio::test]
async fn a_missing_caller_gets_the_auth_error_envelope(#[case] uri: &str) {
    let reply = get(app(None), None, uri).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let body: ErrorEnvelope = reply.json();
    assert_eq!(body.error.message, "the API key is not valid");
    assert_eq!(body.error.error_type, "auth_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "401");
}

#[rstest]
#[case::distinct("/tag/distinct")]
#[case::daily("/tag/dau")]
#[case::weekly("/tag/wau")]
#[tokio::test]
async fn a_missing_database_answers_the_nested_detail(#[case] uri: &str) {
    let reply = get(app(None), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: NestedBody = reply.json();
    assert_eq!(body.detail.error, DB_DETAIL);
    assert_eq!(
        body.detail.error,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[rstest]
#[case::distinct("/tag/distinct", "Failed to fetch distinct user agent tags")]
#[case::daily("/tag/dau", "Failed to fetch DAU analytics")]
#[case::weekly("/tag/wau", "Failed to fetch WAU analytics")]
#[tokio::test]
async fn a_failed_query_answers_the_report_prefix(#[case] uri: &str, #[case] prefix: &str) {
    let store = Arc::new(FakeStore::failing());
    let reply = get(app(Some(store)), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.json();
    assert_eq!(body.detail, format!("{prefix}: {QUERY_FAILURE}"));
}
