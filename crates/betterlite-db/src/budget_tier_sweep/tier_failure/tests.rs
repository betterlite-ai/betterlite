#![cfg(test)]

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, ScheduleFault, TestSchedule, finish, insert_tier,
    moment, past, pool, reject_budget_updates, reset_at_of, schema, spend_of,
};
use crate::budget_tier_sweep::tier_cascade::TierCascadeFailure;
use crate::budget_tier_sweep::tier_failure::{cascade_failure_message, failure_metadata};
use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::session::PostgresStore;

#[test]
fn failure_metadata_counts_no_budgets_when_the_cascade_never_collected() {
    let failure = TierCascadeFailure::<ScheduleFault>::Collect(ScheduleFault {
        message: "cannot parse duration".to_owned(),
    });
    let metadata = failure_metadata(&failure);
    assert_eq!(
        metadata["num_budgets_found"],
        Json::Number(Number::from(0_u8))
    );
    assert_eq!(
        metadata["num_endusers_found"],
        Json::Number(Number::from(0_u8))
    );
    assert_eq!(cascade_failure_message(&failure), "cannot parse duration");
}

#[test]
fn failure_metadata_keeps_the_collected_budget_count_on_a_commit_error() {
    let failure = TierCascadeFailure::<ScheduleFault>::Commit {
        budgets: 3,
        error: DbError {
            error_type: ErrorType::Other,
            message: "write failed".to_owned(),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
            cause: None,
        },
    };
    let metadata = failure_metadata(&failure);
    assert_eq!(
        metadata["num_budgets_found"],
        Json::Number(Number::from(3_u8))
    );
    assert_eq!(
        metadata["num_endusers_found"],
        Json::Number(Number::from(0_u8))
    );
    assert_eq!(cascade_failure_message(&failure), "write failed");
}

#[tokio::test]
async fn schedule_failure_stops_the_phase_and_leaves_the_tier_due() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::failing(moment(30), "1d", "cannot parse duration"),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let failures = harness.failures.events();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].call_type, "reset_budget_endusers");
    assert_eq!(failures[0].service, "reset_budget_job");
    assert!(failures[0].error.contains("cannot parse duration"));
    assert_eq!(
        failures[0].event_metadata["num_budgets_found"],
        Json::Number(Number::from(0_u8))
    );
    assert!(harness.successes.events().is_empty());
    assert_eq!(reset_at_of(&client, "tier-a").await, Some(past()));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn commit_failure_reports_the_budget_count_and_rolls_the_chunk_back() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-a', 'team-a', 'tier-a', 10)",
        )
        .await
        .unwrap();
    reject_budget_updates(&client).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let failures = harness.failures.events();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].event_metadata["num_budgets_found"],
        Json::Number(Number::from(1_u8))
    );
    assert_eq!(
        failures[0].event_metadata["num_endusers_found"],
        Json::Number(Number::from(0_u8))
    );
    assert!(harness.successes.events().is_empty());
    assert_eq!(reset_at_of(&client, "tier-a").await, Some(past()));
    assert_eq!(
        spend_of(&client, "LiteLLM_TeamMembership", "user_id", "user-a").await,
        Decimal::from(10_i32)
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn read_failure_reports_the_empty_cascade_after_one_reconnect() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    let closed = PostgresStore {
        pool: Some(pool(&fixture.options.url)),
    };
    closed.pool.as_ref().unwrap().close();
    let mut harness = Harness::new(
        closed,
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(false),
    );
    harness.run().await;
    let failures = harness.failures.events();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].event_metadata["num_budgets_found"],
        Json::Number(Number::from(0_u8))
    );
    assert!(failures[0].error.contains("database pool unavailable"));
    assert_eq!(harness.reconnect.as_ref().unwrap().attempts(), 1);
    assert!(harness.successes.events().is_empty());
    finish(fixture, store, client).await;
}
