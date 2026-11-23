use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::support::{Events, Failure, Fake, aggregate, as_json, decimal, json, list_row, seen};
use crate::spend::log_results::projection::{
    SpendLogListRow, SpendLogsPage, build_ui_spend_logs_response,
};
use crate::spend::log_results::session::SessionAggregateError;

fn blank_page(rows: Vec<SpendLogListRow>) -> SpendLogsPage {
    SpendLogsPage {
        data: rows,
        total: 2,
        page: 1,
        page_size: 50,
        total_pages: 1,
        total_is_capped: false,
    }
}

async fn build(reader: Option<&Fake>, page: SpendLogsPage, enrich: bool) -> SpendLogsPage {
    build_ui_spend_logs_response(reader, page, enrich)
        .await
        .unwrap()
}

fn object(value: &JsonData) -> &BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = value else {
        panic!("the value is not an object");
    };
    fields
}

#[tokio::test]
async fn identical_session_ids_stay_separate_per_key() {
    let mut second = aggregate("s1", "k2");
    second.session_total_count = 9;
    second.session_total_spend = decimal("9.5");
    // a key the page never shows must not reach either row, even with the same session id
    let mut stranger = aggregate("s1", "k3");
    stranger.session_total_count = 99;
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: vec![aggregate("s1", "k1"), second, stranger],
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let events = fake.events.clone();
    let rows = vec![
        list_row("r1", Some("s1"), Some("k1")),
        list_row("r2", Some("s1"), Some("k2")),
    ];
    let built = build(Some(&fake), blank_page(rows), true).await;
    assert_eq!(
        seen(&events),
        [String::from("aggregate sessions=s1 keys=k1|k2")]
    );
    assert_eq!(built.data[0].session_total_count, Some(3));
    assert_eq!(built.data[0].session_total_spend, Some(decimal("1.5")));
    assert_eq!(built.data[1].session_total_count, Some(9));
    assert_eq!(built.data[1].session_total_spend, Some(decimal("9.5")));
}

#[tokio::test]
async fn the_lookup_holds_truthy_ids_and_non_null_keys_once() {
    let fake = Fake::default();
    let events = fake.events.clone();
    let rows = vec![
        list_row("r1", Some("s1"), Some("k1")),
        list_row("r2", Some("s1"), Some("k1")),
        list_row("r3", Some(""), Some("k2")),
        list_row("r4", None, Some("k1")),
        list_row("r5", Some("s2"), None),
    ];
    let built = build(Some(&fake), blank_page(rows), true).await;
    assert_eq!(
        seen(&events),
        [String::from("aggregate sessions=s1|s2 keys=k1|k2")]
    );
    assert_eq!(built.data.len(), 5);
}

#[tokio::test]
async fn rows_without_a_session_id_skip_the_query() {
    let fake = Fake::default();
    let events = fake.events.clone();
    let rows = vec![
        list_row("r1", None, Some("k1")),
        list_row("r2", Some(""), Some("k1")),
    ];
    let built = build(Some(&fake), blank_page(rows), true).await;
    assert!(seen(&events).is_empty());
    assert_eq!(built.data[0].session_total_count, Some(1));
    assert_eq!(built.data[1].session_total_count, Some(1));
    assert_eq!(built.data[0].session_total_spend, None);
}

#[tokio::test]
async fn a_row_without_stats_counts_one_call_and_invents_no_fields() {
    let fake = Fake::default();
    let events = fake.events.clone();
    let built = build(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    assert_eq!(seen(&events).len(), 1);
    let row = &built.data[0];
    assert_eq!(row.session_total_count, Some(1));
    assert_eq!(row.session_total_spend, None);
    assert_eq!(row.session_total_duration_ms, None);
    assert_eq!(row.mcp_tool_call_count, None);
    assert_eq!(row.session_models, None);
    let fields = object(&as_json(row));
    assert_eq!(fields.get("session_total_count"), Some(&json("1")));
    assert_eq!(fields.get("session_total_spend"), None);
    assert_eq!(fields.get("session_models_truncated"), None);
}

#[tokio::test]
async fn stats_fill_every_session_field() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: vec![aggregate("s1", "k1")],
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let built = build(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    let row = &built.data[0];
    assert_eq!(row.session_total_count, Some(3));
    assert_eq!(row.session_total_spend, Some(decimal("1.5")));
    assert_eq!(row.session_total_duration_ms, Some(1_200));
    assert_eq!(row.session_cache_hit_count, Some(1));
    assert_eq!(row.session_llm_count, Some(2));
    assert_eq!(row.session_agent_count, Some(1));
    assert_eq!(row.session_total_prompt_tokens, Some(10));
    assert_eq!(row.session_completion_tokens, Some(20));
    assert_eq!(row.session_total_tokens, Some(30));
    assert_eq!(row.session_models, Some(vec![String::from("m")]));
    assert_eq!(row.session_models_truncated, Some(false));
    let fields = object(&as_json(row));
    assert_eq!(
        fields.get("session_total_completion_tokens"),
        Some(&json("20"))
    );
}

#[tokio::test]
async fn a_zero_mcp_count_adds_neither_mcp_field() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: vec![aggregate("s1", "k1")],
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let built = build(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    let row = &built.data[0];
    assert_eq!(row.mcp_tool_call_count, None);
    assert_eq!(row.mcp_tool_call_spend, None);
    let fields = object(&as_json(row));
    assert_eq!(fields.get("mcp_tool_call_count"), None);
    assert_eq!(fields.get("mcp_tool_call_spend"), None);
}

#[tokio::test]
async fn a_nonzero_mcp_count_adds_both_mcp_fields() {
    let mut stats = aggregate("s1", "k1");
    stats.mcp_tool_call_count = 2;
    stats.mcp_tool_call_spend = decimal("0.5");
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: vec![stats],
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let built = build(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    let row = &built.data[0];
    assert_eq!(row.mcp_tool_call_count, Some(2));
    assert_eq!(row.mcp_tool_call_spend, Some(decimal("0.5")));
    let fields = object(&as_json(row));
    assert_eq!(fields.get("mcp_tool_call_count"), Some(&json("2")));
    assert_eq!(fields.get("mcp_tool_call_spend"), Some(&json("0.5")));
}

#[tokio::test]
async fn a_database_failure_falls_back_to_plain_counts() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: Some(Failure::Database),
    };
    let built = build(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    assert_eq!(built.data[0].session_total_count, Some(1));
    assert_eq!(built.data[0].session_total_spend, None);
}

#[tokio::test]
async fn a_malformed_aggregate_row_propagates() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: Some(Failure::Row),
    };
    let result = build_ui_spend_logs_response(
        Some(&fake),
        blank_page(vec![list_row("r1", Some("s1"), Some("k1"))]),
        true,
    )
    .await;
    assert!(matches!(result, Err(SessionAggregateError::Row(_))));
}

#[tokio::test]
async fn no_reader_leaves_plain_counts() {
    let rows = vec![list_row("r1", Some("s1"), Some("k1"))];
    let built = build(None, blank_page(rows), true).await;
    assert_eq!(built.data[0].session_total_count, Some(1));
    assert_eq!(built.data[0].session_total_spend, None);
}

#[tokio::test]
async fn public_v2_skips_the_query_and_keeps_the_envelope() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: vec![aggregate("s1", "k1")],
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let events = fake.events.clone();
    let rows = vec![list_row("r1", Some("s1"), Some("k1"))];
    let built = build(Some(&fake), blank_page(rows), false).await;
    assert!(seen(&events).is_empty());
    assert_eq!(built.data[0].session_total_count, None);
    assert_eq!(built.data[0].session_total_spend, None);
    let row_fields = object(&as_json(&built.data[0]));
    assert_eq!(row_fields.get("session_total_count"), None);
    let fields = object(&as_json(&built));
    assert!(fields.contains_key("data"));
    assert_eq!(fields.get("total"), Some(&json("2")));
    assert_eq!(fields.get("page"), Some(&json("1")));
    assert_eq!(fields.get("page_size"), Some(&json("50")));
    assert_eq!(fields.get("total_pages"), Some(&json("1")));
    assert_eq!(fields.get("total_is_capped"), Some(&json("false")));
}
