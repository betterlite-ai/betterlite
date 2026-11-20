#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::rows::ActiveUsersReport;
use crate::user_agent_analytics::rows::ActiveUsersRow;
use crate::user_agent_analytics::tests::support::DetailBody;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::NestedBody;
use crate::user_agent_analytics::tests::support::Reply;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::app_at;
use crate::user_agent_analytics::tests::support::at;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::close;
use crate::user_agent_analytics::tests::support::database;
use crate::user_agent_analytics::tests::support::get;
use crate::user_agent_analytics::tests::support::postgres_app;
use crate::user_agent_analytics::tests::support::seed;

const MAU: &str = "/tag/mau";
const QUERY_FAILURE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

const INSERT_MONTH_ROWS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-c', 'user-3'), ('sk-d', 'user-4');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-11', 'sk-a', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2025-12-12', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:alpha', '2025-06-16', 'sk-d', CURRENT_TIMESTAMP),
 ('5', 'User-Agent:alpha', '2025-06-15', 'sk-a', CURRENT_TIMESTAMP),
 ('6', 'User-Agent:alpha', '2025-07-15', 'sk-b', CURRENT_TIMESTAMP),
 ('7', 'User-Agent:beta', '2026-01-05', 'sk-c', CURRENT_TIMESTAMP),
 ('8', 'User-Agent:beta', '2026-01-06', 'sk-d', CURRENT_TIMESTAMP);"#;

async fn ask(store: Option<&Arc<FakeStore>>, uri: &str) -> Reply {
    get(app(store.map(Arc::clone)), Some(&caller()), uri).await
}

fn row(offset: i64, active_users: i64) -> ActiveUsersRow {
    ActiveUsersRow {
        tag: "User-Agent:alpha".to_owned(),
        active_users,
        date: offset.to_string(),
    }
}

#[tokio::test]
async fn monthly_report_asks_for_the_two_hundred_and_nine_days_before_tomorrow() {
    let store = Arc::new(FakeStore::empty());
    let reply = ask(Some(&store), MAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_call();
    assert_eq!(recorded.report, ActiveUsersReport::Monthly);
    assert_eq!(recorded.start_date, "2025-06-16");
    assert_eq!(recorded.end_date, "2026-01-11");
    assert_eq!(recorded.filter, None);
}

#[tokio::test]
async fn monthly_report_ends_at_the_next_utc_midnight() {
    let late = at(2026, 1, 31, 23, 59);
    let store = Arc::new(FakeStore::empty());
    let reply = get(app_at(Some(Arc::clone(&store)), late), Some(&caller()), MAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_call();
    assert_eq!(recorded.start_date, "2025-07-07");
    assert_eq!(recorded.end_date, "2026-02-01");
}

#[tokio::test]
async fn monthly_report_labels_every_thirty_day_period() {
    let rows: Vec<ActiveUsersRow> = (0_i64..7).rev().map(|offset| row(offset, 1)).collect();
    let store = Arc::new(FakeStore::empty().with_active(rows));
    let reply = ask(Some(&store), MAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","active_users":1,"date":"Month 1 (Jun)","#,
            r#""period_start":"2025-06-16","period_end":"2025-07-15"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 2 (Jul)","#,
            r#""period_start":"2025-07-16","period_end":"2025-08-14"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 3 (Aug)","#,
            r#""period_start":"2025-08-15","period_end":"2025-09-13"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 4 (Sep)","#,
            r#""period_start":"2025-09-14","period_end":"2025-10-13"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 5 (Oct)","#,
            r#""period_start":"2025-10-14","period_end":"2025-11-12"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 6 (Nov)","#,
            r#""period_start":"2025-11-13","period_end":"2025-12-12"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 7 (Dec)","#,
            r#""period_start":"2025-12-13","period_end":"2026-01-11"}]}"#
        )
    );
}

#[rstest]
#[case::negative("-1")]
#[case::past_the_seventh_month("7")]
#[case::beyond_the_calendar("9223372036854775807")]
#[case::not_a_number("not-a-month")]
#[tokio::test]
async fn monthly_report_rejects_an_impossible_offset(#[case] offset: &str) {
    let mut broken = row(0, 1);
    broken.date = offset.to_owned();
    let store = Arc::new(FakeStore::empty().with_active(vec![broken]));
    let reply = ask(Some(&store), MAU).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.json();
    assert!(body.detail.starts_with("Failed to fetch MAU analytics: "));
}

#[rstest]
#[case::absent("", None)]
#[case::exact("?tag_filters=alpha&tag_filters=beta", Some(TagFilter::Exact(vec!["alpha".to_owned(), "beta".to_owned()])))]
#[case::substring("?tag_filter=alpha", Some(TagFilter::Substring("alpha".to_owned())))]
#[case::exact_precedes("?tag_filters=alpha&tag_filter=beta", Some(TagFilter::Exact(vec!["alpha".to_owned()])))]
#[tokio::test]
async fn monthly_report_forwards_the_decoded_tag_filter(
    #[case] query: &str,
    #[case] expected: Option<TagFilter>,
) {
    let store = Arc::new(FakeStore::empty());
    let uri = format!("{MAU}{query}");
    let reply = ask(Some(&store), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.only_call().filter, expected);
}

#[tokio::test]
async fn monthly_report_guards_the_endpoint() {
    let unauthorized = get(app(None), None, MAU).await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    let missing = get(app(None), Some(&caller()), MAU).await;
    assert_eq!(missing.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: NestedBody = missing.json();
    assert_eq!(
        body.detail.error,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    let failing = Arc::new(FakeStore::failing());
    let reply = ask(Some(&failing), MAU).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let failure: DetailBody = reply.json();
    assert_eq!(
        failure.detail,
        format!("Failed to fetch MAU analytics: {QUERY_FAILURE}")
    );
}

#[tokio::test]
async fn monthly_report_buckets_and_filters_rows() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_MONTH_ROWS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), MAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","active_users":2,"date":"Month 1 (Jun)","#,
            r#""period_start":"2025-06-16","period_end":"2025-07-15"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 6 (Nov)","#,
            r#""period_start":"2025-11-13","period_end":"2025-12-12"},"#,
            r#"{"tag":"User-Agent:beta","active_users":2,"date":"Month 7 (Dec)","#,
            r#""period_start":"2025-12-13","period_end":"2026-01-11"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"date":"Month 7 (Dec)","#,
            r#""period_start":"2025-12-13","period_end":"2026-01-11"}]}"#
        )
    );
    let exact = "/tag/mau?tag_filters=User-Agent%3Aalpha";
    let exact_reply = get(postgres_app(&pool), Some(&caller()), exact).await;
    assert!(!exact_reply.text().contains("User-Agent:beta"));
    let upper = get(
        postgres_app(&pool),
        Some(&caller()),
        "/tag/mau?tag_filter=ALPHA",
    )
    .await;
    assert!(upper.text().contains("User-Agent:alpha"));
    close(fixture, pool).await;
}
