#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::rows::TagSpendRecord;
use crate::user_agent_analytics::rows::TokenOwnerRow;
use crate::user_agent_analytics::rows::UserEmailRow;
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

const PER_USER: &str = "/tag/user-agent/per-user-analytics";
const QUERY_FAILURE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

const INSERT_PER_USER_ROWS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-null', NULL);
INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
 ('user-1', 'u1@x'), ('user-2', NULL);
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, api_requests, successful_requests, failed_requests, prompt_tokens, completion_tokens, spend, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2025-12-20', 'sk-a', 2, 2, 0, 10, 5, 0.5, CURRENT_TIMESTAMP),
 ('2', 'User-Agent:beta', '2025-12-21', 'sk-a', 1, 1, 0, 3, 2, 0.25, CURRENT_TIMESTAMP),
 ('3', 'User-Agent:gamma', '2025-12-22', 'sk-b', 4, 3, 1, 0, 0, 1.0, CURRENT_TIMESTAMP),
 ('4', 'User-Agent:alpha', '2025-12-11', 'sk-a', 9, 9, 0, 0, 0, 9.0, CURRENT_TIMESTAMP),
 ('5', 'User-Agent:alpha', '2025-12-23', 'sk-unknown', 5, 5, 0, 0, 0, 5.0, CURRENT_TIMESTAMP),
 ('6', 'User-Agent:alpha', '2025-12-24', 'sk-null', 6, 6, 0, 0, 0, 6.0, CURRENT_TIMESTAMP),
 ('7', 'User-Agent:Alpha', '2025-12-25', 'sk-b', 0, 0, 0, 0, 0, 0.0, CURRENT_TIMESTAMP);"#;

#[derive(serde::Deserialize)]
struct PerUserBody {
    results: Vec<PerUserRow>,
    total_count: i64,
    total_pages: i64,
}

#[derive(serde::Deserialize)]
struct PerUserRow {
    user_id: String,
    user_email: Option<String>,
    user_agent: Option<String>,
    successful_requests: i64,
    failed_requests: i64,
    total_requests: i64,
    total_tokens: i64,
    #[serde(with = "rust_decimal::serde::float")]
    spend: Decimal,
}

async fn ask(store: Option<&Arc<FakeStore>>, uri: &str) -> Reply {
    get(app(store.map(Arc::clone)), Some(&caller()), uri).await
}

fn record(api_key: &str, tag: &str, successful: i64) -> TagSpendRecord {
    TagSpendRecord {
        tag: Some(tag.to_owned()),
        api_key: api_key.to_owned(),
        api_requests: successful,
        successful_requests: successful,
        failed_requests: 0,
        prompt_tokens: 10,
        completion_tokens: 5,
        spend: Decimal::new(1, 1),
    }
}

fn two_users() -> Arc<FakeStore> {
    let mut sparse = record("sk-b", "User-Agent:beta", 0);
    sparse.tag = None;
    sparse.prompt_tokens = 0;
    sparse.completion_tokens = 0;
    sparse.spend = Decimal::ZERO;
    Arc::new(
        FakeStore::empty()
            .with_records(vec![
                record("sk-a", "User-Agent:alpha", 3),
                sparse,
                record("sk-a", "User-Agent:gamma", 1),
                record("sk-b", "User-Agent:beta", 0),
                record("", "User-Agent:alpha", 9),
            ])
            .with_owners(vec![
                TokenOwnerRow {
                    token: "sk-a".to_owned(),
                    user_id: "u1".to_owned(),
                },
                TokenOwnerRow {
                    token: "sk-b".to_owned(),
                    user_id: "u2".to_owned(),
                },
            ])
            .with_users(vec![UserEmailRow {
                user_id: "u1".to_owned(),
                user_email: Some("m@x".to_owned()),
            }]),
    )
}

fn metrics_of(body: &PerUserBody) -> Vec<(&str, i64, i64, i64, i64, Decimal)> {
    body.results
        .iter()
        .map(|row| {
            (
                row.user_id.as_str(),
                row.successful_requests,
                row.failed_requests,
                row.total_requests,
                row.total_tokens,
                row.spend,
            )
        })
        .collect()
}

fn identities_of(body: &PerUserBody) -> Vec<(&str, Option<&str>, Option<&str>)> {
    body.results
        .iter()
        .map(|row| {
            (
                row.user_id.as_str(),
                row.user_email.as_deref(),
                row.user_agent.as_deref(),
            )
        })
        .collect()
}

fn ids(body: &PerUserBody) -> Vec<&str> {
    body.results
        .iter()
        .map(|row| row.user_id.as_str())
        .collect()
}

#[tokio::test]
async fn per_user_report_returns_zero_pages_without_any_api_key() {
    let store = Arc::new(FakeStore::empty());
    let reply = ask(Some(&store), PER_USER).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        r#"{"results":[],"total_count":0,"page":1,"page_size":50,"total_pages":0}"#
    );
    let recorded = store.only_records();
    assert_eq!(recorded.start_date, "2025-12-12");
    assert_eq!(recorded.end_date, "2026-01-11");
    assert!(store.token_calls().is_empty());
    assert!(store.user_calls().is_empty());
}

#[tokio::test]
async fn per_user_report_aggregates_each_users_records() {
    let store = two_users();
    let reply = ask(Some(&store), PER_USER).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: PerUserBody = reply.json();
    assert_eq!(
        metrics_of(&body),
        vec![
            ("u1", 4, 0, 4, 30, Decimal::new(2, 1)),
            ("u2", 0, 0, 0, 15, Decimal::new(1, 1)),
        ]
    );
    assert_eq!(
        identities_of(&body),
        vec![
            ("u1", Some("m@x"), Some("User-Agent:alpha")),
            ("u2", None, Some("User-Agent:beta")),
        ]
    );
    assert_eq!((body.total_count, body.total_pages), (2, 1));
    assert_eq!(
        store.token_calls(),
        vec![vec!["sk-a".to_owned(), "sk-b".to_owned()]]
    );
    assert_eq!(
        store.user_calls(),
        vec![vec!["u1".to_owned(), "u2".to_owned()]]
    );
}

#[rstest]
#[case::first_page("page=1&page_size=1", &["u1"], 2)]
#[case::second_page("page=2&page_size=1", &["u2"], 2)]
#[case::past_the_last_page("page=3&page_size=1", &[], 2)]
#[case::largest_page_size("page_size=1000", &["u1", "u2"], 1)]
#[case::padded_page("page=%202&page_size=1", &["u2"], 2)]
#[tokio::test]
async fn per_user_report_paginates_the_sorted_users(
    #[case] query: &str,
    #[case] expected: &[&str],
    #[case] expected_pages: i64,
) {
    let uri = format!("{PER_USER}?{query}");
    let reply = ask(Some(&two_users()), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: PerUserBody = reply.json();
    assert_eq!(ids(&body), expected);
    assert_eq!(body.total_count, 2);
    assert_eq!(body.total_pages, expected_pages);
}

#[rstest]
#[case::zero_page("page=0", r#"{"detail":[{"type":"greater_than_equal","loc":["query","page"],"msg":"Input should be greater than or equal to 1"}]}"#)]
#[case::zero_size("page_size=0", r#"{"detail":[{"type":"greater_than_equal","loc":["query","page_size"],"msg":"Input should be greater than or equal to 1"}]}"#)]
#[case::large_size("page_size=1001", r#"{"detail":[{"type":"less_than_equal","loc":["query","page_size"],"msg":"Input should be less than or equal to 1000"}]}"#)]
#[case::not_a_number("page=abc", r#"{"detail":[{"type":"int_parsing","loc":["query","page"],"msg":"Input should be a valid integer, unable to parse string as an integer"}]}"#)]
#[case::beyond_the_integer("page=9223372036854775808", r#"{"detail":[{"type":"int_parsing","loc":["query","page"],"msg":"Input should be a valid integer, unable to parse string as an integer"}]}"#)]
#[tokio::test]
async fn per_user_report_rejects_an_invalid_page(#[case] query: &str, #[case] expected: &str) {
    let uri = format!("{PER_USER}?{query}");
    let reply = get(app(None), Some(&caller()), &uri).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.text(), expected);
}

#[rstest]
#[case::absent("", None)]
#[case::exact("?tag_filters=alpha&tag_filters=beta", Some(TagFilter::Exact(vec!["alpha".to_owned(), "beta".to_owned()])))]
#[case::substring("?tag_filter=alpha", Some(TagFilter::Substring("alpha".to_owned())))]
#[tokio::test]
async fn per_user_report_forwards_the_decoded_tag_filter(
    #[case] query: &str,
    #[case] expected: Option<TagFilter>,
) {
    let store = Arc::new(FakeStore::empty());
    let uri = format!("{PER_USER}{query}");
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.only_records().filter, expected);
}

#[tokio::test]
async fn per_user_report_guards_the_endpoint() {
    let unauthorized = get(app(None), None, PER_USER).await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    let missing = get(app(None), Some(&caller()), PER_USER).await;
    assert_eq!(missing.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: NestedBody = missing.json();
    assert_eq!(
        body.detail.error,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    let failing = Arc::new(FakeStore::failing());
    let reply = ask(Some(&failing), PER_USER).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let failure: DetailBody = reply.json();
    assert_eq!(
        failure.detail,
        format!("Failed to fetch per-user analytics: {QUERY_FAILURE}")
    );
}

#[tokio::test]
async fn per_user_report_maps_keys_to_users() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_PER_USER_ROWS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), PER_USER).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: PerUserBody = reply.json();
    assert_eq!(
        metrics_of(&body),
        vec![
            ("user-1", 3, 0, 3, 20, Decimal::new(75, 2)),
            ("user-2", 3, 1, 4, 0, Decimal::new(1, 0)),
        ]
    );
    assert_eq!(
        identities_of(&body),
        vec![
            ("user-1", Some("u1@x"), Some("User-Agent:alpha")),
            ("user-2", None, Some("User-Agent:gamma")),
        ]
    );
    assert_eq!((body.total_count, body.total_pages), (2, 1));
    close(fixture, pool).await;
}

#[tokio::test]
async fn per_user_report_filters_records_by_tag() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_PER_USER_ROWS).await;
    let exact = get(
        postgres_app(&pool),
        Some(&caller()),
        "/tag/user-agent/per-user-analytics?tag_filters=User-Agent%3Aalpha",
    )
    .await;
    let exact_body: PerUserBody = exact.json();
    assert_eq!(ids(&exact_body), vec!["user-1"]);
    let lower = get(
        postgres_app(&pool),
        Some(&caller()),
        "/tag/user-agent/per-user-analytics?tag_filter=alpha",
    )
    .await;
    let lower_body: PerUserBody = lower.json();
    assert_eq!(ids(&lower_body), vec!["user-1"]);
    let upper = get(
        postgres_app(&pool),
        Some(&caller()),
        "/tag/user-agent/per-user-analytics?tag_filter=Alpha",
    )
    .await;
    let upper_body: PerUserBody = upper.json();
    assert_eq!(ids(&upper_body), vec!["user-2"]);
    close(fixture, pool).await;
}

#[tokio::test]
async fn per_user_report_folds_several_keys_into_one_user() {
    let store = Arc::new(
        FakeStore::empty()
            .with_records(vec![
                record("sk-a", "User-Agent:alpha", 1),
                record("sk-b", "User-Agent:beta", 2),
            ])
            .with_owners(vec![
                TokenOwnerRow {
                    token: "sk-a".to_owned(),
                    user_id: "u1".to_owned(),
                },
                TokenOwnerRow {
                    token: "sk-b".to_owned(),
                    user_id: "u1".to_owned(),
                },
            ]),
    );
    let reply = ask(Some(&store), PER_USER).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: PerUserBody = reply.json();
    assert_eq!(
        metrics_of(&body),
        vec![("u1", 3, 0, 3, 30, Decimal::new(2, 1))]
    );
    assert_eq!(
        identities_of(&body),
        vec![("u1", None, Some("User-Agent:alpha"))]
    );
    assert_eq!((body.total_count, body.total_pages), (1, 1));
    assert_eq!(
        store.token_calls(),
        vec![vec!["sk-a".to_owned(), "sk-b".to_owned()]]
    );
    assert_eq!(store.user_calls(), vec![vec!["u1".to_owned()]]);
}

#[tokio::test]
async fn per_user_report_reports_an_aggregate_overflow() {
    let mut first = record("sk-a", "User-Agent:alpha", 1);
    first.spend = Decimal::MAX;
    let mut second = record("sk-a", "User-Agent:beta", 1);
    second.spend = Decimal::MAX;
    let store = Arc::new(
        FakeStore::empty()
            .with_records(vec![first, second])
            .with_owners(vec![TokenOwnerRow {
                token: "sk-a".to_owned(),
                user_id: "u1".to_owned(),
            }]),
    );
    let reply = ask(Some(&store), PER_USER).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let failure: DetailBody = reply.json();
    assert_eq!(
        failure.detail,
        "Failed to fetch per-user analytics: the analytics totals exceed the supported range"
    );
}
