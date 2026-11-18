#![cfg(test)]

use rust_decimal::Decimal;

use super::spend_counter_keys;
use crate::budget_reset_policy::cache_invalidation::InvalidatedCache;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, assert_spend, finish, moment, naive,
    reset_at_of, schema, write,
};

const FAIL_TRIGGER: &str = r#"
CREATE FUNCTION fail_key_update() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'write rejected'; END; $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_key_update BEFORE UPDATE ON "LiteLLM_VerificationToken" FOR EACH ROW EXECUTE FUNCTION fail_key_update();
"#;

#[test]
fn counter_invalidation_names_the_litellm_spend_entries() {
    let writes = vec![
        write("first", None, Decimal::ZERO),
        write("second", None, Decimal::ZERO),
    ];
    assert_eq!(
        spend_counter_keys(IdentityPhase::Keys, &writes),
        vec!["spend:key:first".to_owned(), "spend:key:second".to_owned()]
    );
    assert_eq!(
        spend_counter_keys(IdentityPhase::Users, &writes),
        vec![
            "spend:user:first".to_owned(),
            "spend:user:second".to_owned()
        ]
    );
}

#[test]
fn counter_invalidation_skips_an_empty_chunk() {
    assert!(spend_counter_keys(IdentityPhase::Keys, &[]).is_empty());
    assert!(spend_counter_keys(IdentityPhase::Users, &[]).is_empty());
}

#[tokio::test]
async fn counter_invalidation_clears_the_spend_counter_after_the_commit() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('u1', 10, '1d', NULL);",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Users).await;
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "u1", 0.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_UserTable", "user_id", "u1").await,
        Some(naive(30))
    );
    let calls = harness.cache.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(calls[0].keys, vec!["spend:user:u1".to_owned()]);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn counter_invalidation_waits_for_a_committed_write() {
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
    assert!(harness.cache.calls().is_empty());
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 10.0).await;
    assert_eq!(harness.failures.events().len(), 1);
    assert!(harness.successes.events().is_empty());
    finish(fixture, store, client).await;
}
