#![cfg(test)]

use rust_decimal::Decimal;

use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, finish, insert_tier, moment, naive,
    past, reset_at_of, schema, spend_of,
};

#[tokio::test]
async fn cascade_advances_the_tier_through_the_schedule() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("30d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-a', 'team-a', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_eq!(harness.schedule.durations(), vec!["30d".to_owned()]);
    assert_eq!(reset_at_of(&client, "tier-a").await, Some(naive(30)));
    assert_eq!(
        spend_of(&client, "LiteLLM_TeamMembership", "user_id", "user-a").await,
        Decimal::ZERO
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn cascade_carries_spend_past_the_cap_when_rollover_is_enabled() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, max_budget, budget_duration, budget_reset_at) VALUES ('tier-a', 10, '1d', '2025-12-30 00:00:00')",
        )
        .await
        .unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-a', 'team-a', 'tier-a', 15)",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.rollover = RolloverPolicy { enabled: true };
    harness.run().await;
    assert_eq!(
        spend_of(&client, "LiteLLM_TeamMembership", "user_id", "user-a").await,
        Decimal::from(5)
    );
    finish(fixture, store, client).await;
}
