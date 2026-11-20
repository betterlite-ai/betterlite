#![cfg(test)]

use betterlite_models::decisions::Json;
use deadpool_postgres::Client;
use serde_json::Number;

use crate::budget_reset_policy::cache_invalidation::InvalidatedCache;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::phase_failure::EventMetadata;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::team_window_reset::tests::support::{
    CountingReconnect, RecordingCache, TABLES, TEAM_COLUMN, TEAM_TABLE, TeamHarness, TestSchedule,
    assert_spend, close_store, finish, insert_team, moment, past, provision, reset_at_of, schema,
    spend_of,
};

const TRIGGER: &str = r#"
CREATE FUNCTION reject_team_reset() RETURNS trigger AS $$
BEGIN RAISE EXCEPTION 'budget updates are frozen'; END; $$ LANGUAGE plpgsql;
CREATE TRIGGER reject_team_reset BEFORE UPDATE ON "LiteLLM_TeamTable"
FOR EACH ROW EXECUTE FUNCTION reject_team_reset();
"#;

async fn reset_count(client: &Client, identifiers: &[&str]) -> usize {
    let mut total = 0_usize;
    for identifier in identifiers {
        let spent = spend_of(client, TEAM_TABLE, TEAM_COLUMN, identifier).await;
        if spent.to_bits() == 0.0_f64.to_bits() {
            total = total.saturating_add(1);
        }
    }
    total
}

fn number_of(event_metadata: &EventMetadata, key: &str) -> Option<u64> {
    match event_metadata.get(key) {
        Some(Json::Number(number)) => number.as_u64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[tokio::test]
async fn resets_every_due_team_and_reports_the_counts() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0), Some("1h"), Some(past())).await;
    insert_team(&client, "b", 4.5, None, Some("1d"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 0.0).await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "b", 0.0).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "a").await,
        Some(moment(3600).naive_utc())
    );
    let events = harness.successes.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert_eq!(event.call_type, "reset_budget_teams");
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.start_time, moment(0));
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(2));
    assert_eq!(
        number_of(&event.event_metadata, "num_teams_updated"),
        Some(2)
    );
    assert_eq!(
        number_of(&event.event_metadata, "num_teams_failed"),
        Some(0)
    );
    assert!(harness.failures.events().is_empty());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn leaves_a_team_whose_reset_time_is_ahead_alone() {
    let (fixture, store, client) = schema().await;
    let fresh_at = moment(600).naive_utc();
    insert_team(&client, "fresh", 1.0, None, Some("1h"), Some(fresh_at)).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "fresh", 1.0).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "fresh").await,
        Some(fresh_at)
    );
    assert_eq!(
        number_of(
            &harness.successes.events().first().unwrap().event_metadata,
            "num_teams_found"
        ),
        Some(0)
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn invalidates_each_reset_teams_counter() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0), Some("1h"), Some(past())).await;
    insert_team(&client, "b", 4.5, None, Some("1d"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let calls = harness.cache.calls();
    assert_eq!(calls.len(), 1_usize);
    let call = calls.first().unwrap();
    assert_eq!(call.sink, InvalidatedCache::SpendCounter);
    let mut keys = call.keys.clone();
    keys.sort();
    assert_eq!(
        keys,
        vec!["spend:team:a".to_owned(), "spend:team:b".to_owned()]
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn rollover_keeps_the_spend_above_the_cap() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 12.5, Some(10.0), Some("1h"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.rollover = RolloverPolicy { enabled: true };
    harness.run().await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 2.5).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "a").await,
        Some(moment(3600).naive_utc())
    );
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn one_refused_team_does_not_stop_the_chunk() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "bad", 10.0, Some(20.0), Some("1h"), Some(past())).await;
    insert_team(&client, "good", 4.0, None, Some("1d"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::failing_for(moment(3600), "1h", "no such unit"),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "bad", 10.0).await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "good", 0.0).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "bad").await,
        Some(past())
    );
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "good").await,
        Some(moment(3600).naive_utc())
    );
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert_eq!(event.call_type, "reset_budget_teams");
    assert_eq!(event.error, "Failed to reset 1 teams: bad: no such unit");
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(2));
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn a_chunk_where_every_team_fails_reports_them_without_writing() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 10.0, Some(20.0), Some("1h"), Some(past())).await;
    insert_team(&client, "b", 4.0, None, Some("1h"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::failing_for(moment(3600), "1h", "no such unit"),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 10.0).await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "b", 4.0).await;
    assert_eq!(
        reset_at_of(&client, TEAM_TABLE, TEAM_COLUMN, "a").await,
        Some(past())
    );
    assert!(harness.cache.calls().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert!(event.error.starts_with("Failed to reset 2 teams: "));
    assert!(event.error.contains("a: no such unit"));
    assert!(event.error.contains("b: no such unit"));
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(2));
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn a_failed_read_reports_the_phase_without_writing() {
    let (fixture, store, client) = provision(vec!["SELECT 1".to_owned()]).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert_eq!(event.call_type, "reset_budget_teams");
    assert!(event.error.contains("LiteLLM_TeamTable"));
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(0));
    assert!(harness.cache.calls().is_empty());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn a_failed_write_reports_the_phase_and_stops() {
    let (fixture, store, client) = provision(vec![TABLES.to_owned(), TRIGGER.to_owned()]).await;
    insert_team(&client, "a", 10.0, Some(20.0), Some("1h"), Some(past())).await;
    insert_team(&client, "b", 4.0, None, Some("1d"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert_eq!(event.call_type, "reset_budget_teams");
    assert!(event.error.contains("budget updates are frozen"));
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(2));
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "a", 10.0).await;
    assert_spend(&client, TEAM_TABLE, TEAM_COLUMN, "b", 4.0).await;
    assert!(harness.cache.calls().is_empty());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn chunks_stop_at_the_configured_maximum() {
    let (fixture, store, client) = schema().await;
    insert_team(&client, "a", 1.0, None, Some("1h"), Some(past())).await;
    insert_team(&client, "b", 2.0, None, Some("1h"), Some(past())).await;
    insert_team(&client, "c", 3.0, None, Some("1h"), Some(past())).await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 2,
    };
    harness.run().await;
    assert_eq!(reset_count(&client, &["a", "b", "c"]).await, 2_usize);
    assert_eq!(harness.successes.events().len(), 2_usize);
    assert_eq!(harness.cache.calls().len(), 2_usize);
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 100,
    };
    harness.run().await;
    assert_eq!(reset_count(&client, &["a", "b", "c"]).await, 3_usize);
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[tokio::test]
async fn nothing_due_reports_zero_counts_and_writes_nothing() {
    let (fixture, store, client) = schema().await;
    let mut harness = TeamHarness::new(
        store,
        TestSchedule::fixed(moment(3600)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let events = harness.successes.events();
    assert_eq!(events.len(), 1_usize);
    let event = events.first().unwrap();
    assert_eq!(event.call_type, "reset_budget_teams");
    assert_eq!(number_of(&event.event_metadata, "num_teams_found"), Some(0));
    assert_eq!(
        number_of(&event.event_metadata, "num_teams_updated"),
        Some(0)
    );
    assert_eq!(
        number_of(&event.event_metadata, "num_teams_failed"),
        Some(0)
    );
    assert!(harness.failures.events().is_empty());
    assert!(harness.cache.calls().is_empty());
    close_store(harness.store.clone());
    finish(fixture, harness.store, client).await;
}

#[test]
fn number_metadata_is_read_only_from_numbers() {
    let mut metadata = EventMetadata::new();
    metadata.insert("k", Json::Text("2".to_owned()));
    assert_eq!(number_of(&metadata, "k"), None);
    assert_eq!(number_of(&metadata, "missing"), None);
    metadata.insert("n", Json::Number(Number::from(3_u64)));
    assert_eq!(number_of(&metadata, "n"), Some(3));
}
