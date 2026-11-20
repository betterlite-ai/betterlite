#![cfg(test)]

use std::collections::BTreeMap;

use deadpool_postgres::Client;

use crate::budget_permission_rows::JsonData;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::team_window_reset::tests::support::{
    CountingReconnect, FakeCounterCache, KEY_COLUMN, KEY_TABLE, TABLES, TEAM_COLUMN, TEAM_TABLE,
    TestDurations, TestSchedule, WindowHarness, budget_limits_type, close_store, decimal, finish,
    insert_expired_windows, insert_serialized_windows, insert_window_row, insert_windows, moment,
    next_text, past, provision, schema, window_json, window_text, windows_text,
};

const PAST_TEXT: &str = "2025-12-30T00:00:00+00:00";
const FRESH_TEXT: &str = "2026-01-01T00:10:00+00:00";

const TEAM_ONLY_TABLES: &str = r#"
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    budget_limits JSONB,
    spend DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    budget_duration TEXT,
    budget_reset_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_BudgetWindowSpend" (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    window_duration TEXT NOT NULL,
    window_start TIMESTAMP(3) NOT NULL,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL,
    PRIMARY KEY (entity_type, entity_id, window_duration)
);
"#;

fn bad_window() -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert(
        "budget_duration".to_owned(),
        JsonData::Number(serde_json::Number::from(5_u32)),
    );
    fields.insert(
        "reset_at".to_owned(),
        JsonData::String(PAST_TEXT.to_owned()),
    );
    JsonData::Object(fields)
}

async fn only_window(client: &Client, table: &str, column: &str, id: &str) -> JsonData {
    let mut windows = windows_text(client, table, column, id).await;
    assert_eq!(windows.len(), 1_usize);
    windows.remove(0)
}

async fn assert_window_reset(client: &Client, table: &str, column: &str, id: &str) {
    assert_eq!(
        budget_limits_type(client, table, column, id).await,
        "string"
    );
    let window = only_window(client, table, column, id).await;
    assert_eq!(window_text(&window, "reset_at"), Some(next_text()));
}

#[tokio::test]
async fn walks_every_page_until_the_table_ends() {
    let (fixture, store, client) = schema().await;
    insert_expired_windows(&client, KEY_TABLE, KEY_COLUMN, &["a", "b", "c"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 2,
        max_chunks_per_run: 100,
    };
    harness.run().await;
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "a").await;
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "b").await;
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "c").await;
    assert_eq!(harness.clock.calls(), 1_usize);
    assert_eq!(
        harness.cache.memory_of("spend:key:c:window:1h"),
        Some(decimal("0"))
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn leaves_a_window_that_has_not_expired_alone() {
    let (fixture, store, client) = schema().await;
    insert_window_row(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "1-fresh",
        window_json("1h", None, Some(FRESH_TEXT)),
    )
    .await;
    insert_expired_windows(&client, KEY_TABLE, KEY_COLUMN, &["2-due"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 100,
    };
    harness.run().await;
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "1-fresh").await,
        "array"
    );
    let fresh = only_window(&client, KEY_TABLE, KEY_COLUMN, "1-fresh").await;
    assert_eq!(window_text(&fresh, "reset_at"), Some(FRESH_TEXT.to_owned()));
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "2-due").await;
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn reads_and_keeps_the_json_string_form() {
    let (fixture, store, client) = schema().await;
    insert_serialized_windows(
        &client,
        KEY_TABLE,
        KEY_COLUMN,
        "s",
        &[window_json("1h", None, Some(PAST_TEXT))],
    )
    .await;
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "s").await,
        "string"
    );
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "s").await;
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn rewrites_the_whole_list_keeping_the_fresh_window() {
    let (fixture, store, client) = schema().await;
    let windows = vec![
        window_json("1h", None, Some(PAST_TEXT)),
        window_json("1d", None, Some(FRESH_TEXT)),
    ];
    insert_windows(&client, KEY_TABLE, KEY_COLUMN, "k1", &windows).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let stored = windows_text(&client, KEY_TABLE, KEY_COLUMN, "k1").await;
    assert_eq!(stored.len(), 2_usize);
    let hourly = stored.first().unwrap();
    let daily = stored.get(1).unwrap();
    assert_eq!(
        window_text(hourly, "budget_duration"),
        Some("1h".to_owned())
    );
    assert_eq!(window_text(daily, "budget_duration"), Some("1d".to_owned()));
    assert_eq!(window_text(hourly, "reset_at"), Some(next_text()));
    assert_eq!(window_text(daily, "reset_at"), Some(FRESH_TEXT.to_owned()));
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn resets_team_windows_too() {
    let (fixture, store, client) = schema().await;
    insert_expired_windows(&client, KEY_TABLE, KEY_COLUMN, &["k1"]).await;
    insert_expired_windows(&client, TEAM_TABLE, TEAM_COLUMN, &["t1"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_window_reset(&client, KEY_TABLE, KEY_COLUMN, "k1").await;
    assert_window_reset(&client, TEAM_TABLE, TEAM_COLUMN, "t1").await;
    assert_eq!(
        harness.cache.memory_of("spend:team:t1:window:1h"),
        Some(decimal("0"))
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn a_malformed_window_stops_only_its_own_source() {
    let (fixture, store, client) = schema().await;
    insert_windows(&client, KEY_TABLE, KEY_COLUMN, "1-bad", &[bad_window()]).await;
    insert_expired_windows(&client, KEY_TABLE, KEY_COLUMN, &["2-good"]).await;
    insert_expired_windows(&client, TEAM_TABLE, TEAM_COLUMN, &["t1"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 100,
    };
    harness.run().await;
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "1-bad").await,
        "array"
    );
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "2-good").await,
        "array"
    );
    assert_window_reset(&client, TEAM_TABLE, TEAM_COLUMN, "t1").await;
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn a_failed_write_stops_only_its_own_source() {
    let trigger = "CREATE FUNCTION reject_key_windows() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'key windows are frozen'; END; $$ LANGUAGE plpgsql;
             CREATE TRIGGER reject_key_windows BEFORE UPDATE ON \"LiteLLM_VerificationToken\"
             FOR EACH ROW EXECUTE FUNCTION reject_key_windows();";
    let (fixture, store, client) = provision(vec![TABLES.to_owned(), trigger.to_owned()]).await;
    insert_expired_windows(&client, KEY_TABLE, KEY_COLUMN, &["k1"]).await;
    insert_expired_windows(&client, TEAM_TABLE, TEAM_COLUMN, &["t1"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_eq!(
        budget_limits_type(&client, KEY_TABLE, KEY_COLUMN, "k1").await,
        "array"
    );
    assert_window_reset(&client, TEAM_TABLE, TEAM_COLUMN, "t1").await;
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}

#[tokio::test]
async fn a_missing_table_stops_only_its_own_source() {
    let (fixture, store, client) = provision(vec![TEAM_ONLY_TABLES.to_owned()]).await;
    insert_expired_windows(&client, TEAM_TABLE, TEAM_COLUMN, &["t1"]).await;
    let mut harness = WindowHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        FakeCounterCache::default(),
        TestDurations::fixed(3600),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_window_reset(&client, TEAM_TABLE, TEAM_COLUMN, "t1").await;
    close_store(harness.store.clone());
    finish(fixture, harness.store.clone(), client).await;
}
