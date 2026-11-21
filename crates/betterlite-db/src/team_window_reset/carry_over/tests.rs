#![cfg(test)]

use rstest::rstest;

use super::window_carried_spend;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::team_window_reset::tests::support::{FakeCounterCache, decimal};

const COUNTER: &str = "spend:key:k1:window:1h";

#[rstest]
#[case::below_the_cap("4", "5", "0")]
#[case::at_the_cap("5", "5", "0")]
#[case::above_the_cap("10", "4", "6")]
#[case::overdrawn("0", "5", "0")]
#[case::zero_cap("3", "0", "3")]
#[tokio::test]
async fn carry_over_keeps_the_spend_above_the_window_cap(
    #[case] counter: &str,
    #[case] cap: &str,
    #[case] carried: &str,
) {
    let cache = FakeCounterCache::with_counter(COUNTER, Some(decimal(counter)));
    let value = window_carried_spend(
        Some(decimal(cap)),
        COUNTER,
        RolloverPolicy { enabled: true },
        &cache,
    )
    .await;
    assert_eq!(value, decimal(carried));
    assert_eq!(cache.read_keys(), vec![COUNTER.to_owned()]);
}

#[tokio::test]
async fn carry_over_is_zero_without_rollover_and_reads_nothing() {
    let cache = FakeCounterCache::with_counter(COUNTER, Some(decimal("10")));
    let value = window_carried_spend(
        Some(decimal("4")),
        COUNTER,
        RolloverPolicy { enabled: false },
        &cache,
    )
    .await;
    assert_eq!(value, decimal("0"));
    assert!(cache.read_keys().is_empty());
}

#[tokio::test]
async fn carry_over_is_zero_without_a_window_cap() {
    let cache = FakeCounterCache::with_counter(COUNTER, Some(decimal("10")));
    let value = window_carried_spend(None, COUNTER, RolloverPolicy { enabled: true }, &cache).await;
    assert_eq!(value, decimal("0"));
    assert!(cache.read_keys().is_empty());
}

#[tokio::test]
async fn carry_over_falls_back_to_zero_when_the_counter_cannot_be_read() {
    let cache = FakeCounterCache::failing_reads();
    let value = window_carried_spend(
        Some(decimal("4")),
        COUNTER,
        RolloverPolicy { enabled: true },
        &cache,
    )
    .await;
    assert_eq!(value, decimal("0"));
}

#[tokio::test]
async fn carry_over_falls_back_to_zero_when_the_counter_is_not_a_number() {
    let cache = FakeCounterCache::with_counter(COUNTER, None);
    let value = window_carried_spend(
        Some(decimal("4")),
        COUNTER,
        RolloverPolicy { enabled: true },
        &cache,
    )
    .await;
    assert_eq!(value, decimal("0"));
    assert_eq!(cache.read_keys(), vec![COUNTER.to_owned()]);
}

#[tokio::test]
async fn carry_over_falls_back_to_zero_when_the_counter_is_absent() {
    let cache = FakeCounterCache::default();
    let value = window_carried_spend(
        Some(decimal("4")),
        COUNTER,
        RolloverPolicy { enabled: true },
        &cache,
    )
    .await;
    assert_eq!(value, decimal("0"));
}
