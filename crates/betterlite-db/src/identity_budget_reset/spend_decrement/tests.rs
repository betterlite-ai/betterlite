#![cfg(test)]

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::reset_due_row;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::rows::{DueIdentity, read_due};
use crate::identity_budget_reset::tests::support::{TestSchedule, finish, moment, schema};

fn identity(
    identifier: &str,
    spend: Decimal,
    max_budget: Option<Decimal>,
    budget_reset_at: Option<DateTime<Utc>>,
) -> DueIdentity {
    DueIdentity {
        identifier: identifier.to_owned(),
        spend,
        max_budget,
        budget_duration: "1d".to_owned(),
        budget_reset_at,
    }
}

#[test]
fn spend_decrement_is_the_whole_spend_without_rollover() {
    let schedule = TestSchedule::fixed(moment(30));
    let row = identity("key-a", Decimal::new(125, 1), Some(Decimal::from(10)), None);
    let write = reset_due_row(&row, RolloverPolicy::default(), &schedule).unwrap();
    assert_eq!(write.identifier, "key-a");
    assert_eq!(write.spend_decrement, Decimal::new(125, 1));
    assert_eq!(write.budget_reset_at, Some(moment(30)));
}

#[test]
fn spend_decrement_leaves_the_carried_spend_in_place_with_rollover() {
    let schedule = TestSchedule::fixed(moment(30));
    let row = identity(
        "key-a",
        Decimal::from(30),
        Some(Decimal::from(10)),
        Some(moment(5)),
    );
    let write = reset_due_row(&row, RolloverPolicy { enabled: true }, &schedule).unwrap();
    assert_eq!(write.spend_decrement, Decimal::from(10));
    assert_eq!(write.budget_reset_at, Some(moment(30)));
}

#[test]
fn spend_decrement_carries_nothing_without_a_cap() {
    let schedule = TestSchedule::fixed(moment(30));
    let row = identity("key-a", Decimal::from(30), None, None);
    let write = reset_due_row(&row, RolloverPolicy { enabled: true }, &schedule).unwrap();
    assert_eq!(write.spend_decrement, Decimal::from(30));
}

#[test]
fn spend_decrement_propagates_a_schedule_failure() {
    let schedule = TestSchedule::failing_for(moment(30), "1d", "the duration cannot be parsed");
    let row = identity("key-a", Decimal::from(30), None, None);
    let error = reset_due_row(&row, RolloverPolicy::default(), &schedule).unwrap_err();
    assert_eq!(error.message, "the duration cannot be parsed");
}

#[tokio::test]
async fn spend_decrement_uses_the_float_spend_read_from_the_database() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, max_budget, budget_duration, budget_reset_at) VALUES ('key-a', 12.5, 10.5, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let due = read_due(&store, IdentityPhase::Keys, moment(0), 10)
        .await
        .unwrap();
    let schedule = TestSchedule::fixed(moment(30));
    let write = reset_due_row(&due[0], RolloverPolicy::default(), &schedule).unwrap();
    assert_eq!(write.spend_decrement, Decimal::new(125, 1));
    finish(fixture, store, client).await;
}
