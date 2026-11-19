use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::postgres_store::errors::DbError;

use crate::user_agent_analytics::error::AnalyticsError;
use crate::user_agent_analytics::rows::DistinctTagRow;
use crate::user_agent_analytics::store::UserAgentAnalyticsStore as _;
use crate::user_agent_analytics::tests::support::DistinctBody;
use crate::user_agent_analytics::tests::support::DistinctTagBody;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::get;
use crate::user_agent_analytics::tests::support::pool_less_store;

fn tag(tag: &str) -> DistinctTagRow {
    DistinctTagRow {
        tag: tag.to_owned(),
    }
}

#[tokio::test]
async fn the_distinct_report_answers_the_exact_wire_body() {
    let store = Arc::new(FakeStore {
        distinct: vec![tag("User-Agent:alpha"), tag("plain")],
        ..FakeStore::empty()
    });
    let reply = get(
        app(Some(Arc::clone(&store))),
        Some(&caller()),
        "/tag/distinct",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha"},"#,
            r#"{"tag":"plain"}]}"#
        )
    );
    assert_eq!(store.distinct_calls(), 1);
}

#[tokio::test]
async fn the_distinct_report_keeps_the_order_the_store_returns() {
    let store = Arc::new(FakeStore {
        distinct: vec![tag("User-Agent:beta"), tag("User-Agent:alpha")],
        ..FakeStore::empty()
    });
    let reply = get(app(Some(store)), Some(&caller()), "/tag/distinct").await;
    let body: DistinctBody = reply.json();
    assert_eq!(
        body,
        DistinctBody {
            results: vec![
                DistinctTagBody {
                    tag: "User-Agent:beta".to_owned(),
                },
                DistinctTagBody {
                    tag: "User-Agent:alpha".to_owned(),
                },
            ],
        }
    );
}

#[tokio::test]
async fn the_distinct_report_answers_an_empty_results_list() {
    let reply = get(
        app(Some(Arc::new(FakeStore::empty()))),
        Some(&caller()),
        "/tag/distinct",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"results":[]}"#);
}

#[tokio::test]
async fn the_distinct_report_ignores_query_parameters() {
    let store = Arc::new(FakeStore {
        distinct: vec![tag("plain")],
        ..FakeStore::empty()
    });
    let reply = get(
        app(Some(Arc::clone(&store))),
        Some(&caller()),
        "/tag/distinct?tag_filter=alpha&tag_filters=beta",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"results":[{"tag":"plain"}]}"#);
    assert!(store.calls().is_empty());
    assert_eq!(store.distinct_calls(), 1);
}

#[tokio::test]
async fn a_store_without_a_pool_reports_no_db_connection() {
    let error = pool_less_store()
        .distinct_tags()
        .await
        .expect_err("no pool");
    assert!(matches!(
        error,
        AnalyticsError::Database(DbError::NoDbConnected)
    ));
}
