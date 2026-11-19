#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::{found_metadata, outcome_metadata, report_phase_success};
use crate::budget_reset_policy::phase_failure::{PhaseTiming, RESET_BUDGET_JOB_SERVICE};
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, RecordingSuccesses, TestClock, TestSchedule,
    finish, moment, schema,
};
use crate::postgres_store::session::PostgresStore;

const FAIL_TRIGGER: &str = r#"
CREATE FUNCTION fail_key_update() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'write rejected'; END; $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_key_update BEFORE UPDATE ON "LiteLLM_VerificationToken" FOR EACH ROW EXECUTE FUNCTION fail_key_update();
"#;

fn count(value: u8) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn service_hooks_success_reports_the_service_call_type_and_timing() {
    let sink = RecordingSuccesses::default();
    let timing = PhaseTiming {
        start_time: moment(1),
        end_time: moment(4),
    };
    report_phase_success(
        &sink,
        "reset_budget_keys",
        timing,
        outcome_metadata(IdentityPhase::Keys, 4, 3, 1),
    );
    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.service, RESET_BUDGET_JOB_SERVICE);
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.call_type, "reset_budget_keys");
    assert_eq!(event.start_time, moment(1));
    assert_eq!(event.end_time, moment(4));
    assert_eq!(event.duration_seconds.to_bits(), 3.0_f64.to_bits());
    assert_eq!(
        event.event_metadata,
        outcome_metadata(IdentityPhase::Keys, 4, 3, 1)
    );
}

#[test]
fn service_hooks_metadata_names_the_counts_of_each_phase() {
    assert_eq!(
        found_metadata(IdentityPhase::Keys, 2),
        BTreeMap::from([("num_keys_found", count(2))])
    );
    assert_eq!(
        found_metadata(IdentityPhase::Users, 2),
        BTreeMap::from([("num_users_found", count(2))])
    );
    assert_eq!(
        outcome_metadata(IdentityPhase::Keys, 5, 3, 2),
        BTreeMap::from([
            ("num_keys_failed", count(2)),
            ("num_keys_found", count(5)),
            ("num_keys_updated", count(3)),
        ])
    );
    assert_eq!(
        outcome_metadata(IdentityPhase::Users, 5, 3, 2),
        BTreeMap::from([
            ("num_users_failed", count(2)),
            ("num_users_found", count(5)),
            ("num_users_updated", count(3)),
        ])
    );
}

#[tokio::test]
async fn service_hooks_success_carries_the_chunk_counts() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('u1', 10, '1d', NULL), ('u2', 10, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.clock = TestClock::scripted(vec![moment(1), moment(2), moment(3)], moment(3));
    harness.run(IdentityPhase::Users).await;
    assert!(harness.failures.events().is_empty());
    let events = harness.successes.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.call_type, "reset_budget_users");
    assert_eq!(event.duration_seconds.to_bits(), 1.0_f64.to_bits());
    assert_eq!(
        event.event_metadata,
        BTreeMap::from([
            ("num_users_failed", count(0)),
            ("num_users_found", count(2)),
            ("num_users_updated", count(2)),
        ])
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn service_hooks_success_for_an_empty_chunk_reports_zero_counts() {
    let (fixture, store, client) = schema().await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Keys).await;
    let events = harness.successes.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_metadata,
        BTreeMap::from([
            ("num_keys_failed", count(0)),
            ("num_keys_found", count(0)),
            ("num_keys_updated", count(0)),
        ])
    );
    assert!(harness.cache.calls().is_empty());
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn service_hooks_read_failure_reports_no_rows_found() {
    let store = PostgresStore { pool: None };
    let mut harness = Harness::new(
        store,
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Keys).await;
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.call_type, "reset_budget_keys");
    assert!(event.error.contains("No DB Connected"));
    assert_eq!(
        event.event_metadata,
        BTreeMap::from([("num_keys_found", count(0))])
    );
    let reconnect = harness.reconnect.as_ref().unwrap();
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(
        reconnect.reasons(),
        vec!["reset_budget_read_keys_failure".to_owned()]
    );
}

#[tokio::test]
async fn service_hooks_write_failure_reports_the_rows_found() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(FAIL_TRIGGER).await.unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('k1', 10, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Keys).await;
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.call_type, "reset_budget_keys");
    assert!(event.error.contains("database SQLSTATE"));
    assert_eq!(
        event.event_metadata,
        BTreeMap::from([("num_keys_found", count(1))])
    );
    finish(fixture, store, client).await;
}
