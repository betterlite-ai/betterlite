#![cfg(test)]

use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, assert_spend, finish, moment, naive,
    reset_at_of, schema,
};

#[tokio::test]
async fn service_reset_scopes_each_phase_to_its_own_table() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('k1', 10, '1d', '2025-12-30 00:00:00'); INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('u1', 10, '1d', NULL);",
        )
        .await
        .unwrap();
    let mut keys = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    keys.run(IdentityPhase::Keys).await;
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "k1", 0.0).await;
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "u1", 10.0).await;
    let mut users = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    users.run(IdentityPhase::Users).await;
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "u1", 0.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_UserTable", "user_id", "u1").await,
        Some(naive(30))
    );
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "k1").await,
        Some(naive(30))
    );
    finish(fixture, store, client).await;
}
