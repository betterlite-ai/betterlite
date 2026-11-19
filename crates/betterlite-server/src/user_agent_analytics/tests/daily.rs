use std::sync::Arc;

use axum::http::StatusCode;
use rstest::rstest;

use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::rows::ActiveUsersReport;
use crate::user_agent_analytics::rows::ActiveUsersRow;
use crate::user_agent_analytics::tests::support::ActiveUsersBody;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::app_at;
use crate::user_agent_analytics::tests::support::at;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::get;

const DAU: &str = "/tag/dau";

fn row(tag: &str, active_users: i64, date: &str) -> ActiveUsersRow {
    ActiveUsersRow {
        tag: tag.to_owned(),
        active_users,
        date: date.to_owned(),
    }
}

#[tokio::test]
async fn the_daily_report_asks_for_the_seven_days_before_utc_tomorrow() {
    let store = Arc::new(FakeStore::empty());
    let reply = get(app(Some(Arc::clone(&store))), Some(&caller()), DAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_call();
    assert_eq!(recorded.report, ActiveUsersReport::Daily);
    assert_eq!(recorded.start_date, "2026-01-04");
    assert_eq!(recorded.end_date, "2026-01-11");
    assert_eq!(recorded.filter, None);
}

#[tokio::test]
async fn the_daily_report_ends_at_the_next_utc_midnight() {
    let store = Arc::new(FakeStore::empty());
    let late = at(2026, 1, 31, 23, 59);
    let reply = get(app_at(Some(Arc::clone(&store)), late), Some(&caller()), DAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_call();
    assert_eq!(recorded.report, ActiveUsersReport::Daily);
    assert_eq!(recorded.start_date, "2026-01-25");
    assert_eq!(recorded.end_date, "2026-02-01");
    assert_eq!(recorded.filter, None);
}

#[tokio::test]
async fn the_daily_report_answers_the_exact_wire_body() {
    let store = Arc::new(FakeStore {
        active: vec![
            row("User-Agent:alpha", 2, "2026-01-11"),
            row("User-Agent:alpha", 1, "2026-01-10"),
        ],
        ..FakeStore::empty()
    });
    let reply = get(app(Some(store)), Some(&caller()), DAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","active_users":2,"#,
            r#""date":"2026-01-11","period_start":null,"period_end":null},"#,
            r#"{"tag":"User-Agent:alpha","active_users":1,"#,
            r#""date":"2026-01-10","period_start":null,"period_end":null}]}"#
        )
    );
}

#[tokio::test]
async fn the_daily_report_keeps_the_order_the_store_returns() {
    let store = Arc::new(FakeStore {
        active: vec![
            row("User-Agent:beta", 4, "2026-01-11"),
            row("User-Agent:alpha", 9, "2026-01-10"),
        ],
        ..FakeStore::empty()
    });
    let reply = get(app(Some(store)), Some(&caller()), DAU).await;
    let body: ActiveUsersBody = reply.json();
    let pairs: Vec<(&str, i64, &str)> = body
        .results
        .iter()
        .map(|entry| (entry.tag.as_str(), entry.active_users, entry.date.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("User-Agent:beta", 4, "2026-01-11"),
            ("User-Agent:alpha", 9, "2026-01-10"),
        ]
    );
}

#[rstest]
#[case::absent("", None)]
#[case::substring("?tag_filter=alpha", Some(TagFilter::Substring("alpha".to_owned())))]
#[case::decoded("?tag_filter=User-Agent%3A+alpha", Some(TagFilter::Substring("User-Agent: alpha".to_owned())))]
#[case::blank("?tag_filter=", Some(TagFilter::Substring(String::new())))]
#[case::last_value_wins("?tag_filter=alpha&tag_filter=beta", Some(TagFilter::Substring("beta".to_owned())))]
#[case::exact("?tag_filters=alpha&tag_filters=beta", Some(TagFilter::Exact(vec!["alpha".to_owned(), "beta".to_owned()])))]
#[case::exact_precedes("?tag_filters=alpha&tag_filter=beta", Some(TagFilter::Exact(vec!["alpha".to_owned()])))]
#[case::blank_exact("?tag_filters=", Some(TagFilter::Exact(vec![String::new()])))]
#[tokio::test]
async fn the_daily_report_forwards_the_decoded_tag_filter(
    #[case] query: &str,
    #[case] expected: Option<TagFilter>,
) {
    let store = Arc::new(FakeStore::empty());
    let uri = format!("{DAU}{query}");
    let reply = get(app(Some(Arc::clone(&store))), Some(&caller()), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = store.only_call();
    assert_eq!(recorded.report, ActiveUsersReport::Daily);
    assert_eq!(recorded.start_date, "2026-01-04");
    assert_eq!(recorded.end_date, "2026-01-11");
    assert_eq!(recorded.filter, expected);
}

#[tokio::test]
async fn an_empty_store_answers_an_empty_results_list() {
    let reply = get(
        app(Some(Arc::new(FakeStore::empty()))),
        Some(&caller()),
        DAU,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"results":[]}"#);
}
