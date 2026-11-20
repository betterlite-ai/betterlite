#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::rows::TagSummaryRow;
use crate::user_agent_analytics::tests::support::DetailBody;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::NestedBody;
use crate::user_agent_analytics::tests::support::Reply;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::close;
use crate::user_agent_analytics::tests::support::database;
use crate::user_agent_analytics::tests::support::get;
use crate::user_agent_analytics::tests::support::postgres_app;
use crate::user_agent_analytics::tests::support::seed;

const SUMMARY: &str = "/tag/summary";
const RANGE: &str = "start_date=2026-01-01&end_date=2026-01-31";
const QUERY_FAILURE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

const INSERT_SUMMARY_ROWS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-c', NULL);
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, api_requests, successful_requests, failed_requests, prompt_tokens, completion_tokens, spend, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-02', 'sk-a', 3, 2, 1, 100, 50, 0.5, CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-03', 'sk-b', 2, 2, 0, 10, 20, 0.25, CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2026-01-04', 'sk-unknown', 4, 3, 1, 30, 0, 0.125, CURRENT_TIMESTAMP),
 ('4', 'User-Agent:beta', '2026-01-05', 'sk-c', 1, 0, 1, 0, 0, 0.0, CURRENT_TIMESTAMP),
 ('5', 'User-Agent:alpha', '2026-02-01', 'sk-a', 9, 9, 0, 0, 0, 0.0, CURRENT_TIMESTAMP);"#;

async fn ask(store: Option<&Arc<FakeStore>>, uri: &str) -> Reply {
    get(app(store.map(Arc::clone)), Some(&caller()), uri).await
}

fn null_summary() -> TagSummaryRow {
    TagSummaryRow {
        tag: "User-Agent:alpha".to_owned(),
        unique_users: 0,
        total_requests: None,
        successful_requests: None,
        failed_requests: None,
        total_tokens: None,
        total_spend: None,
    }
}

#[tokio::test]
async fn summary_report_requires_both_dates() {
    let reply = get(app(None), Some(&caller()), SUMMARY).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"detail":[{"type":"missing","loc":["query","start_date"],"msg":"Field required"},"#,
            r#"{"type":"missing","loc":["query","end_date"],"msg":"Field required"}]}"#
        )
    );
}

#[rstest]
#[case::bad_month("start_date=2026-13-01&end_date=2026-01-01")]
#[case::bad_day("start_date=2026-01-01&end_date=2026-02-30")]
#[case::blank("start_date=&end_date=2026-01-01")]
#[tokio::test]
async fn summary_report_rejects_an_invalid_date(#[case] query: &str) {
    let uri = format!("{SUMMARY}?{query}");
    let store = Arc::new(FakeStore::empty());
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body: DetailBody = reply.json();
    assert!(
        body.detail
            .starts_with("Invalid date format. Use YYYY-MM-DD: ")
    );
}

#[tokio::test]
async fn summary_report_coalesces_null_aggregates() {
    let store = Arc::new(FakeStore::empty().with_summary(vec![null_summary()]));
    let uri = format!("{SUMMARY}?{RANGE}");
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","unique_users":0,"#,
            r#""total_requests":0,"successful_requests":0,"failed_requests":0,"#,
            r#""total_tokens":0,"total_spend":0.0}]}"#
        )
    );
}

#[tokio::test]
async fn summary_report_forwards_the_dates_and_filter() {
    let store = Arc::new(FakeStore::empty());
    let uri = format!("{SUMMARY}?{RANGE}&tag_filters=alpha&tag_filter=beta");
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_summary();
    assert_eq!(recorded.start_date, "2026-01-01");
    assert_eq!(recorded.end_date, "2026-01-31");
    assert_eq!(
        recorded.filter,
        Some(TagFilter::Exact(vec!["alpha".to_owned()]))
    );
}

#[tokio::test]
async fn summary_report_guards_the_endpoint() {
    let unauthorized = get(app(None), None, SUMMARY).await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    let uri = format!("{SUMMARY}?{RANGE}");
    let missing = get(app(None), Some(&caller()), &uri).await;
    assert_eq!(missing.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: NestedBody = missing.json();
    assert_eq!(
        body.detail.error,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    let broken = "/tag/summary?start_date=not-a-date&end_date=2026-01-01";
    let without_database = get(app(None), Some(&caller()), broken).await;
    assert_eq!(without_database.status, StatusCode::INTERNAL_SERVER_ERROR);
    let store = Arc::new(FakeStore::failing());
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let failure: DetailBody = reply.json();
    assert_eq!(
        failure.detail,
        format!("Failed to fetch tag summary analytics: {QUERY_FAILURE}")
    );
}

#[tokio::test]
async fn summary_report_retains_keys_and_sums() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_SUMMARY_ROWS).await;
    let uri = format!("{SUMMARY}?{RANGE}");
    let reply = get(postgres_app(&pool), Some(&caller()), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","unique_users":2,"total_requests":9,"#,
            r#""successful_requests":7,"failed_requests":2,"total_tokens":210,"total_spend":0.875},"#,
            r#"{"tag":"User-Agent:beta","unique_users":0,"total_requests":1,"#,
            r#""successful_requests":0,"failed_requests":1,"total_tokens":0,"total_spend":0.0}]}"#
        )
    );
    let exact =
        "/tag/summary?start_date=2026-01-01&end_date=2026-01-31&tag_filters=User-Agent%3Aalpha";
    let exact_reply = get(postgres_app(&pool), Some(&caller()), exact).await;
    assert!(!exact_reply.text().contains("User-Agent:beta"));
    let upper = "/tag/summary?start_date=2026-01-01&end_date=2026-01-31&tag_filter=ALPHA";
    let upper_reply = get(postgres_app(&pool), Some(&caller()), upper).await;
    assert!(upper_reply.text().contains("User-Agent:alpha"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn summary_report_uses_the_decimal_serializer() {
    let mut row = null_summary();
    row.tag = "User-Agent:beta".to_owned();
    row.unique_users = 2;
    row.total_requests = Some(5);
    row.successful_requests = Some(4);
    row.failed_requests = Some(1);
    row.total_tokens = Some(300);
    row.total_spend = Some(Decimal::new(125, 2));
    let store = Arc::new(FakeStore::empty().with_summary(vec![row]));
    let uri = format!("{SUMMARY}?{RANGE}");
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:beta","unique_users":2,"total_requests":5,"#,
            r#""successful_requests":4,"failed_requests":1,"total_tokens":300,"total_spend":1.25}]}"#
        )
    );
}
