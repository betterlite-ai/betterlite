use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;

use super::super::grouped::sql::{GroupedPage, page_query};
use super::super::grouped::{
    GroupedSessionRequest, SessionLogsPage, ui_session_grouped_spend_logs,
};
use super::super::params::SessionParam;
use super::super::ports::{SessionGroupRow, SessionsStore, SessionsStoreError};
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::sessions::tests::support::{
    Aggregates, Events, Failure, Fake, aggregate, as_json, group, json, kinds, representative, seen,
};

fn request<'query>(
    conditions: &'query [String],
    params: Vec<SessionParam>,
    page: i64,
    page_size: i64,
    session_cursor: Option<&'query str>,
) -> GroupedSessionRequest<'query> {
    GroupedSessionRequest {
        conditions,
        params,
        page,
        page_size,
        descending: true,
        session_cursor,
    }
}

fn fake(
    groups: Vec<SessionGroupRow>,
    group_count: i64,
    representatives: Vec<SpendLogListRow>,
) -> Fake {
    Fake {
        events: Events::default(),
        groups,
        group_count,
        representatives,
        rows: Vec::new(),
        row_count: 0,
        failure: None,
    }
}

fn failing() -> Fake {
    Fake {
        events: Events::default(),
        groups: Vec::new(),
        group_count: 0,
        representatives: Vec::new(),
        rows: Vec::new(),
        row_count: 0,
        failure: Some(Failure::Database),
    }
}

async fn page_of(fake: Fake, request: &GroupedSessionRequest<'_>) -> (SessionLogsPage, Events) {
    let events = Arc::clone(&fake.events);
    let store: Arc<dyn SessionsStore> = Arc::new(fake);
    let page = ui_session_grouped_spend_logs(store.as_ref(), None, request)
        .await
        .unwrap();
    (page, events)
}

#[tokio::test]
async fn a_grouped_page_returns_representatives_in_group_order() {
    let fake = fake(
        vec![
            group("s1", "k", "2026-01-03 00:00:00"),
            group("s2", "k", "2026-01-02 00:00:00"),
            group("s3", "k", "2026-01-01 00:00:00"),
        ],
        3,
        vec![
            representative("r2", "s2", "k", "{}"),
            representative("r1", "s1", "k", "{}"),
        ],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 1, 2, None)).await;
    let ids: Vec<&str> = page
        .page
        .data
        .iter()
        .map(|row| row.request_id.as_str())
        .collect();
    assert_eq!(ids, ["r1", "r2"]);
    assert_eq!(page.page.total, 3);
    assert_eq!(page.page.total_pages, 2);
    assert!(!page.page.total_is_capped);
    assert!(page.has_more);
    assert_eq!(
        page.next_session_cursor.as_deref(),
        Some("2026-01-02 00:00:00|k|s2")
    );
    assert_eq!(kinds(&events), ["groups", "group_count", "representatives"]);
}

// a page that runs out while holding sessions names the total itself, so no count query runs
#[tokio::test]
async fn the_last_page_names_the_total_from_its_offset() {
    let fake = fake(
        vec![group("s3", "k", "2026-01-01 00:00:00")],
        99,
        vec![representative("r3", "s3", "k", "{}")],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 2, 2, None)).await;
    assert_eq!(page.page.total, 3);
    assert!(!page.has_more);
    assert_eq!(page.next_session_cursor, None);
    assert_eq!(kinds(&events), ["groups", "representatives"]);
}

// an empty page starting beyond the end cannot establish the total, so it still counts
#[tokio::test]
async fn an_empty_page_beyond_the_end_counts() {
    let fake = fake(Vec::new(), 4, Vec::new());
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 5, 2, None)).await;
    assert_eq!(page.page.total, 4);
    assert!(!page.has_more);
    assert_eq!(page.next_session_cursor, None);
    assert!(page.page.data.is_empty());
    assert_eq!(kinds(&events), ["groups", "group_count"]);
}

// a first empty page starts inside the list, so it ends it with a total of zero
#[tokio::test]
async fn a_first_empty_page_needs_no_count() {
    let fake = fake(Vec::new(), 0, Vec::new());
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 1, 2, None)).await;
    assert_eq!(page.page.total, 0);
    assert_eq!(page.page.total_pages, 0);
    assert!(!page.has_more);
    assert_eq!(kinds(&events), ["groups"]);
}

// a page starting past the count window runs no group query, and its empty negative window still reports has_more with no cursor
#[tokio::test]
async fn a_page_past_the_count_window_runs_no_group_query() {
    let fake = fake(Vec::new(), 10_001, Vec::new());
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 5_002, 2, None)).await;
    assert!(page.page.data.is_empty());
    assert!(page.has_more);
    assert_eq!(page.next_session_cursor, None);
    assert_eq!(page.page.total, 10_000);
    assert!(page.page.total_is_capped);
    assert_eq!(kinds(&events), ["group_count"]);
}

#[tokio::test]
async fn a_capped_count_sets_the_capped_total() {
    let fake = fake(
        vec![
            group("s1", "k", "2026-01-03 00:00:00"),
            group("s2", "k", "2026-01-02 00:00:00"),
            group("s3", "k", "2026-01-01 00:00:00"),
        ],
        10_001,
        vec![
            representative("r1", "s1", "k", "{}"),
            representative("r2", "s2", "k", "{}"),
        ],
    );
    let (page, _events) = page_of(fake, &request(&[], Vec::new(), 1, 2, None)).await;
    assert_eq!(page.page.total, 10_000);
    assert!(page.page.total_is_capped);
}

// the offset is only bound when the page falls inside the count window
#[tokio::test]
async fn an_offset_page_binds_the_offset_after_the_limit() {
    let fake = fake(
        vec![
            group("s3", "k", "2026-01-01 00:00:00"),
            group("s4", "k", "2025-12-31 00:00:00"),
            group("s5", "k", "2025-12-30 00:00:00"),
        ],
        5,
        vec![
            representative("r3", "s3", "k", "{}"),
            representative("r4", "s4", "k", "{}"),
        ],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 2, 2, None)).await;
    assert!(page.has_more);
    assert_eq!(
        page.next_session_cursor.as_deref(),
        Some("2025-12-31 00:00:00|k|s4")
    );
    let recorded = seen(&events);
    let groups = recorded.first().unwrap();
    assert!(
        groups.contains("params=[Integer(3), Integer(2)]"),
        "{groups}"
    );
    assert!(groups.contains("LIMIT $1 OFFSET $2"), "{groups}");
}

// the page window is trimmed to what the count window can still reach
#[tokio::test]
async fn a_page_limit_is_trimmed_at_the_count_window() {
    let fake = fake(
        vec![
            group("s9", "k", "2026-01-01 00:00:00"),
            group("s10", "k", "2025-12-31 00:00:00"),
        ],
        12_000,
        vec![representative("r9", "s9", "k", "{}")],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 3_334, 3, None)).await;
    assert_eq!(page.page.data.len(), 1);
    assert!(page.has_more);
    assert_eq!(
        page.next_session_cursor.as_deref(),
        Some("2026-01-01 00:00:00|k|s9")
    );
    assert_eq!(page.page.total, 10_000);
    assert!(page.page.total_is_capped);
    let recorded = seen(&events);
    let groups = recorded.first().unwrap();
    assert!(
        groups.contains("params=[Integer(2), Integer(9999)]"),
        "{groups}"
    );
    assert!(groups.contains("LIMIT $1 OFFSET $2"), "{groups}");
}

// the wire cursor carries api_key before session_key, while the SQL tuple binds session first
#[tokio::test]
async fn a_cursor_page_binds_the_tuple_in_sql_order() {
    let conditions = vec![String::from("api_key = $1")];
    let params = vec![SessionParam::Text(String::from("k"))];
    let fake = fake(
        vec![group("session-1", "key-1", "2026-01-02 00:00:00")],
        5,
        vec![representative("r1", "session-1", "key-1", "{}")],
    );
    let cursor = Some("2026-01-03 00:00:00|key-1|session-1");
    let (page, events) = page_of(fake, &request(&conditions, params, 7, 2, cursor)).await;
    assert_eq!(page.page.page, 7);
    assert_eq!(page.page.total, 5);
    assert!(!page.has_more);
    assert_eq!(page.next_session_cursor, None);
    let recorded = seen(&events);
    assert_cursor_sql(recorded.first().unwrap());
    assert_eq!(kinds(&events), ["groups", "group_count", "representatives"]);
}

fn assert_cursor_sql(groups: &str) {
    assert!(
        groups.contains(
            r#"params=[Text("k"), Text("2026-01-03 00:00:00"), Text("session-1"), Text("key-1"), Integer(3)]"#
        ),
        "{groups}"
    );
    assert!(
        groups.contains("HAVING (MAX(\"startTime\"), COALESCE(NULLIF(session_id, ''), request_id), api_key) < ($2::timestamp, $3, $4)"),
        "{groups}"
    );
    assert!(!groups.contains("OFFSET"), "{groups}");
}

#[tokio::test]
async fn an_ascending_page_sorts_the_other_way() {
    let conditions = vec![String::from("TRUE")];
    let fake = fake(
        vec![group("s1", "k", "2026-01-01 00:00:00")],
        1,
        vec![representative("r1", "s1", "k", "{}")],
    );
    let ascending = GroupedSessionRequest {
        conditions: &conditions,
        params: Vec::new(),
        page: 1,
        page_size: 2,
        descending: false,
        session_cursor: Some("2026-01-01 00:00:00|k|s0"),
    };
    let (_page, events) = page_of(fake, &ascending).await;
    let recorded = seen(&events);
    let groups = recorded.first().unwrap();
    assert!(groups.contains(") > ($1::timestamp, $2, $3)"), "{groups}");
    assert!(groups.contains("MAX(\"startTime\") ASC"), "{groups}");
}

#[tokio::test]
async fn representatives_are_hydrated_and_enriched() {
    let fake = fake(
        vec![group("s1", "k", "2026-01-03 00:00:00")],
        1,
        vec![representative("r1", "s1", "k", "{\"status\":\"success\"}")],
    );
    let aggregates = Aggregates {
        events: Events::default(),
        aggregates: vec![aggregate("s1", "k")],
    };
    let aggregate_events = Arc::clone(&aggregates.events);
    let store: Arc<dyn SessionsStore> = Arc::new(fake);
    let page = ui_session_grouped_spend_logs(
        store.as_ref(),
        Some(&aggregates),
        &request(&[], Vec::new(), 1, 50, None),
    )
    .await
    .unwrap();
    let row = page.page.data.first().unwrap();
    let JsonData::Object(fields) = as_json(row) else {
        panic!("the representative is not an object");
    };
    assert_eq!(
        fields.get("metadata"),
        Some(&json("{\"status\":\"success\"}"))
    );
    assert_eq!(
        fields.get("request_tags"),
        Some(&json("\"[{\\\"team\\\":\\\"a\\\"}]\""))
    );
    assert_eq!(fields.get("session_total_count"), Some(&json("1")));
    assert_eq!(fields.get("session_models"), Some(&json("[\"m\"]")));
    assert_eq!(seen(&aggregate_events).len(), 1);
}

// a pair the representative query did not return is skipped instead of invented
#[tokio::test]
async fn a_missing_representative_is_skipped() {
    let fake = fake(
        vec![
            group("s1", "k", "2026-01-03 00:00:00"),
            group("s2", "k", "2026-01-02 00:00:00"),
        ],
        2,
        vec![representative("r2", "s2", "k", "{}")],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 1, 50, None)).await;
    let ids: Vec<&str> = page
        .page
        .data
        .iter()
        .map(|row| row.request_id.as_str())
        .collect();
    assert_eq!(ids, ["r2"]);
    let recorded = seen(&events);
    let representatives = recorded.last().unwrap();
    assert!(
        representatives.contains(r#"params=[Texts(["s1", "s2"]), Texts(["k", "k"])]"#),
        "{representatives}"
    );
}

#[tokio::test]
async fn a_store_failure_propagates() {
    let store: Arc<dyn SessionsStore> = Arc::new(failing());
    let outcome =
        ui_session_grouped_spend_logs(store.as_ref(), None, &request(&[], Vec::new(), 1, 2, None))
            .await;
    assert!(matches!(outcome, Err(SessionsStoreError::Database(_))));
}

// a page answering exactly its limit is the end of the list: only an extra group proves the list continues
#[tokio::test]
async fn a_full_page_without_an_extra_group_ends_the_list() {
    let fake = fake(
        vec![
            group("s2", "k", "2026-01-02 00:00:00"),
            group("s1", "k", "2026-01-01 00:00:00"),
        ],
        9,
        vec![
            representative("r2", "s2", "k", "{}"),
            representative("r1", "s1", "k", "{}"),
        ],
    );
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 1, 2, None)).await;
    assert!(!page.has_more);
    assert_eq!(page.next_session_cursor, None);
    assert_eq!(page.page.total, 2);
    assert!(!page.page.total_is_capped);
    assert_eq!(kinds(&events), ["groups", "representatives"]);
}

// the cap only flags a total it had to clamp, so a count landing exactly on it reports itself
#[tokio::test]
async fn a_count_at_the_cap_stays_uncapped() {
    let fake = fake(Vec::new(), 10_000, Vec::new());
    let (page, events) = page_of(fake, &request(&[], Vec::new(), 5, 2, None)).await;
    assert_eq!(page.page.total, 10_000);
    assert!(!page.page.total_is_capped);
    assert_eq!(kinds(&events), ["groups", "group_count"]);
}

// the offset clause only joins a window that still has a limit to offset
#[test]
fn a_zero_window_binds_no_offset() {
    let page = GroupedPage {
        conditions: &[],
        params: Vec::new(),
        cursor: None,
        descending: true,
        page_limit: 0,
        offset: 3,
    };
    let query = page_query(&page);
    assert!(!query.sql.contains("OFFSET"), "{}", query.sql);
    assert_eq!(query.params, [SessionParam::Integer(1)]);
}
