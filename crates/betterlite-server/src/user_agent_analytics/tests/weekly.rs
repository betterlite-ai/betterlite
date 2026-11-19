use std::sync::Arc;

use axum::http::StatusCode;
use rstest::rstest;

use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::rows::WeeklyActiveUsersRow;
use crate::user_agent_analytics::tests::support::ActiveUserBody;
use crate::user_agent_analytics::tests::support::ActiveUsersBody;
use crate::user_agent_analytics::tests::support::ActiveUsersCall;
use crate::user_agent_analytics::tests::support::FakeStore;
use crate::user_agent_analytics::tests::support::app;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::get;

const WAU: &str = "/tag/wau";

fn row(tag: &str, active_users: i64, week_offset: i64) -> WeeklyActiveUsersRow {
    WeeklyActiveUsersRow {
        tag: tag.to_owned(),
        active_users,
        week_offset,
    }
}

fn call(start_date: &str, end_date: &str, filter: Option<TagFilter>) -> ActiveUsersCall {
    ActiveUsersCall {
        start_date: start_date.to_owned(),
        end_date: end_date.to_owned(),
        filter,
    }
}

fn point(active_users: i64, date: &str, start: &str, end: &str) -> ActiveUserBody {
    ActiveUserBody {
        tag: "User-Agent:alpha".to_owned(),
        active_users,
        date: date.to_owned(),
        period_start: Some(start.to_owned()),
        period_end: Some(end.to_owned()),
    }
}

#[tokio::test]
async fn the_weekly_report_asks_for_the_forty_eight_days_before_utc_tomorrow() {
    let store = Arc::new(FakeStore::empty());
    let reply = get(app(Some(Arc::clone(&store))), Some(&caller()), WAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![call("2025-11-24", "2026-01-11", None)]);
}

#[tokio::test]
async fn the_weekly_report_answers_the_exact_wire_body() {
    let store = Arc::new(FakeStore {
        weekly: vec![row("User-Agent:alpha", 3, 0), row("User-Agent:alpha", 2, 6)],
        ..FakeStore::empty()
    });
    let reply = get(app(Some(store)), Some(&caller()), WAU).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"results":[{"tag":"User-Agent:alpha","active_users":3,"#,
            r#""date":"Week 7 (Jan 05)","period_start":"2026-01-05","#,
            r#""period_end":"2026-01-11"},"#,
            r#"{"tag":"User-Agent:alpha","active_users":2,"#,
            r#""date":"Week 1 (Nov 24)","period_start":"2025-11-24","#,
            r#""period_end":"2025-11-30"}]}"#
        )
    );
}

#[tokio::test]
async fn the_weekly_report_labels_every_week_of_the_window() {
    let store = Arc::new(FakeStore {
        weekly: (0..7)
            .map(|offset| row("User-Agent:alpha", 1, offset))
            .collect(),
        ..FakeStore::empty()
    });
    let reply = get(app(Some(store)), Some(&caller()), WAU).await;
    let body: ActiveUsersBody = reply.json();
    assert_eq!(
        body.results,
        vec![
            point(1, "Week 7 (Jan 05)", "2026-01-05", "2026-01-11"),
            point(1, "Week 6 (Dec 29)", "2025-12-29", "2026-01-04"),
            point(1, "Week 5 (Dec 22)", "2025-12-22", "2025-12-28"),
            point(1, "Week 4 (Dec 15)", "2025-12-15", "2025-12-21"),
            point(1, "Week 3 (Dec 08)", "2025-12-08", "2025-12-14"),
            point(1, "Week 2 (Dec 01)", "2025-12-01", "2025-12-07"),
            point(1, "Week 1 (Nov 24)", "2025-11-24", "2025-11-30"),
        ]
    );
}

#[rstest]
#[case::absent("", None)]
#[case::exact("?tag_filters=alpha&tag_filters=beta", Some(TagFilter::Exact(vec!["alpha".to_owned(), "beta".to_owned()])))]
#[case::substring("?tag_filter=alpha", Some(TagFilter::Substring("alpha".to_owned())))]
#[tokio::test]
async fn the_weekly_report_forwards_the_decoded_tag_filter(
    #[case] query: &str,
    #[case] expected: Option<TagFilter>,
) {
    let store = Arc::new(FakeStore::empty());
    let uri = format!("{WAU}{query}");
    let reply = get(app(Some(Arc::clone(&store))), Some(&caller()), &uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![call("2025-11-24", "2026-01-11", expected)]
    );
}
