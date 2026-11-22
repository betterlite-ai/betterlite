use std::collections::BTreeSet;

use betterlite_redis::request_batch::lock;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::reservation_settlement::adjust::{
    EntryAdjustment, counter_can_apply_adjustment, entry_adjustment, reseed_reserved_entry,
    set_entries_actual_cost,
};
use crate::reservation_settlement::error::SettlementError;
use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::support::{APPLY, DEFER, REJECT, counters, decimal, entry, take_calls, value_of};

fn item(counter_key: &str, adjustment: &str) -> EntryAdjustment {
    EntryAdjustment {
        entry_index: 0,
        counter_key: counter_key.to_owned(),
        target_adjustment: decimal(adjustment),
        adjustment: decimal(adjustment),
    }
}

#[test]
fn an_adjustment_is_the_target_minus_what_was_applied() {
    let mut priced = entry("spend:key:k1", "10");
    priced.applied_adjustment = decimal("-2");
    let found = entry_adjustment(&priced, 3, decimal("4"), decimal("10"))
        .expect("no overflow")
        .expect("a delta");
    let expected = EntryAdjustment {
        entry_index: 3,
        counter_key: "spend:key:k1".to_owned(),
        target_adjustment: decimal("-6"),
        adjustment: decimal("-4"),
    };
    assert_eq!(found, expected);
}

#[test]
fn a_keyless_entry_yields_no_adjustment() {
    let mut keyless = entry("spend:key:k1", "10");
    keyless.counter_key = None;
    assert_eq!(
        entry_adjustment(&keyless, 0, decimal("4"), decimal("10")).expect("no overflow"),
        None
    );
}

#[test]
fn a_settle_that_matches_what_was_applied_yields_no_adjustment() {
    let mut priced = entry("spend:key:k1", "10");
    priced.applied_adjustment = decimal("-6");
    assert_eq!(
        entry_adjustment(&priced, 0, decimal("4"), decimal("10")).expect("no overflow"),
        None
    );
}

#[test]
fn an_entry_without_a_cost_settles_against_the_reservation_cost() {
    let mut unpriced = entry("spend:key:k1", "10");
    unpriced.reserved_cost = None;
    let found = entry_adjustment(&unpriced, 0, decimal("4"), decimal("9"))
        .expect("no overflow")
        .expect("a delta");
    assert_eq!(found.target_adjustment, decimal("-5"));
}

#[test]
fn a_cost_outside_the_decimal_range_is_an_error() {
    let mut priced = entry("spend:key:k1", "0");
    priced.reserved_cost = Some(Decimal::MAX);
    let outcome = entry_adjustment(&priced, 0, Decimal::MIN, Decimal::MAX);
    assert!(matches!(outcome, Err(SettlementError::Overflow)));
}

#[tokio::test]
async fn an_unreadable_counter_is_inconsistent() {
    let mut fake = counters(&[("spend:key:k1", "5")]);
    fake.unreadable.insert("spend:key:k1".to_owned());
    assert!(!counter_can_apply_adjustment(&fake, "spend:key:k1", decimal("1")).await);
}

#[tokio::test]
async fn a_counter_with_no_value_is_inconsistent() {
    let fake = counters(&[]);
    assert!(!counter_can_apply_adjustment(&fake, "spend:key:k1", decimal("1")).await);
}

#[tokio::test]
async fn a_positive_delta_fits_a_readable_counter() {
    let fake = counters(&[("spend:key:k1", "0")]);
    assert!(counter_can_apply_adjustment(&fake, "spend:key:k1", decimal("5")).await);
}

// a positive delta only adds, so it fits even where the sum would not be representable
#[tokio::test]
async fn a_positive_delta_at_the_decimal_ceiling_stays_consistent() {
    let fake = counters(&[]);
    let _previous = lock(&fake.values).insert("spend:key:k1".to_owned(), Decimal::MAX);
    assert!(counter_can_apply_adjustment(&fake, "spend:key:k1", decimal("1")).await);
}

#[rstest]
#[case::exact("5", "-5", true)]
#[case::within_the_floor("5", "-5.000000000001", true)]
#[case::below_the_floor("5", "-5.000000000002", false)]
#[case::from_zero("0", "-2", false)]
#[tokio::test]
async fn the_negative_floor_absorbs_rounding(
    #[case] current: &str,
    #[case] adjustment: &str,
    #[case] expected: bool,
) {
    let fake = counters(&[("spend:key:k1", current)]);
    let can_apply = counter_can_apply_adjustment(&fake, "spend:key:k1", decimal(adjustment)).await;
    assert_eq!(can_apply, expected);
}

#[tokio::test]
async fn a_negative_sum_outside_the_decimal_range_is_inconsistent() {
    let fake = counters(&[]);
    let _previous = lock(&fake.values).insert("spend:key:k1".to_owned(), Decimal::MIN);
    assert!(!counter_can_apply_adjustment(&fake, "spend:key:k1", Decimal::MIN).await);
}

#[tokio::test]
async fn a_reseeded_counter_is_charged_the_actual_cost() {
    let mut fake = counters(&[]);
    fake.reseeded.insert("spend:key:k1".to_owned());
    reseed_reserved_entry(&fake, &item("spend:key:k1", "3"), decimal("3"))
        .await
        .expect("the reseed lands");
    let calls = take_calls(&fake);
    assert_eq!(calls.reseeds, vec!["spend:key:k1".to_owned()]);
    let increments = vec![("spend:key:k1".to_owned(), decimal("3"))];
    assert_eq!(calls.increments, increments);
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("3")));
}

#[tokio::test]
async fn a_counter_without_a_database_floor_is_not_charged() {
    let fake = counters(&[]);
    reseed_reserved_entry(&fake, &item("spend:key:k1", "3"), decimal("3"))
        .await
        .expect("the reseed lands");
    assert!(take_calls(&fake).increments.is_empty());
}

#[tokio::test]
async fn a_zero_actual_cost_needs_no_charge_after_reseed() {
    let mut fake = counters(&[]);
    fake.reseeded.insert("spend:key:k1".to_owned());
    reseed_reserved_entry(&fake, &item("spend:key:k1", "0"), Decimal::ZERO)
        .await
        .expect("the reseed lands");
    assert!(take_calls(&fake).increments.is_empty());
}

#[tokio::test]
async fn consistent_deltas_go_out_in_one_pipeline() {
    let fake = counters(&[("spend:key:k1", "10"), ("spend:key:k2", "10")]);
    let mut entries = vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")];
    let pending = set_entries_actual_cost(&fake, &mut entries, decimal("4"), decimal("10"), APPLY)
        .await
        .expect("the settle");
    assert!(pending.is_empty());
    let calls = take_calls(&fake);
    let expected = vec![vec![
        ("spend:key:k1".to_owned(), decimal("-6")),
        ("spend:key:k2".to_owned(), decimal("-6")),
    ]];
    assert_eq!(calls.pipelines, expected);
    assert_eq!(
        calls.reads,
        vec!["spend:key:k1".to_owned(), "spend:key:k2".to_owned()]
    );
    assert_eq!(entries[0].applied_adjustment, decimal("-6"));
    assert_eq!(entries[1].applied_adjustment, decimal("-6"));
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("4")));
}

#[tokio::test]
async fn a_deferred_settle_returns_the_pending_deltas_unwritten() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let mut entries = vec![entry("spend:key:k1", "10")];
    let pending = set_entries_actual_cost(&fake, &mut entries, decimal("4"), decimal("10"), DEFER)
        .await
        .expect("the settle");
    let expected = vec![PendingSpendIncrement {
        counter_key: "spend:key:k1".to_owned(),
        increment: decimal("-6"),
    }];
    assert_eq!(pending, expected);
    let calls = take_calls(&fake);
    assert!(calls.pipelines.is_empty());
    assert_eq!(entries[0].applied_adjustment, Decimal::ZERO);
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("10")));
}

#[tokio::test]
async fn an_inconsistent_counter_is_reseeded_and_charged() {
    let mut fake = counters(&[("spend:key:k1", "10")]);
    fake.reseeded.insert("spend:key:k2".to_owned());
    let mut entries = vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")];
    set_entries_actual_cost(&fake, &mut entries, decimal("4"), decimal("10"), APPLY)
        .await
        .expect("the settle");
    let calls = take_calls(&fake);
    let pipelined = vec![vec![("spend:key:k1".to_owned(), decimal("-6"))]];
    assert_eq!(calls.pipelines, pipelined);
    assert_eq!(calls.reseeds, vec!["spend:key:k2".to_owned()]);
    assert_eq!(
        calls.increments,
        vec![("spend:key:k2".to_owned(), decimal("4"))]
    );
    assert_eq!(entries[1].applied_adjustment, decimal("-6"));
    assert_eq!(value_of(&fake, "spend:key:k2"), Some(decimal("4")));
}

#[tokio::test]
async fn a_deferred_inconsistent_counter_is_reseeded_and_stamped() {
    let mut fake = counters(&[]);
    fake.reseeded.insert("spend:key:k2".to_owned());
    let mut entries = vec![entry("spend:key:k2", "10")];
    let pending = set_entries_actual_cost(&fake, &mut entries, decimal("4"), decimal("10"), DEFER)
        .await
        .expect("the settle");
    assert!(pending.is_empty());
    assert_eq!(take_calls(&fake).reseeds, vec!["spend:key:k2".to_owned()]);
    assert_eq!(entries[0].applied_adjustment, decimal("-6"));
}

#[tokio::test]
async fn the_batch_scope_covers_every_entry_before_any_adjustment() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let mut keyless = entry("spend:key:k3", "10");
    keyless.counter_key = None;
    let mut entries = vec![
        entry("spend:key:k1", "10"),
        entry("spend:key:k2", "10"),
        keyless,
    ];
    let pending = set_entries_actual_cost(&fake, &mut entries, decimal("10"), decimal("10"), APPLY)
        .await
        .expect("the settle");
    assert!(pending.is_empty());
    let calls = take_calls(&fake);
    let scoped = BTreeSet::from(["spend:key:k1".to_owned(), "spend:key:k2".to_owned()]);
    assert_eq!(calls.scopes, vec![scoped]);
    assert!(calls.reads.is_empty());
    assert!(calls.pipelines.is_empty());
}

#[tokio::test]
async fn a_rejected_resize_names_the_first_inconsistent_counter() {
    let fake = counters(&[("spend:key:k2", "10")]);
    let mut entries = vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")];
    let error = set_entries_actual_cost(&fake, &mut entries, decimal("6"), decimal("10"), REJECT)
        .await
        .expect_err("the refusal");
    assert!(
        matches!(error, SettlementError::InconsistentCounter { ref counter_key } if counter_key == "spend:key:k1")
    );
    let calls = take_calls(&fake);
    assert!(calls.pipelines.is_empty());
    assert!(calls.reseeds.is_empty());
    assert_eq!(entries[0].applied_adjustment, Decimal::ZERO);
}

#[tokio::test]
async fn a_rejected_resize_passes_when_every_counter_is_consistent() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let mut entries = vec![entry("spend:key:k1", "10")];
    set_entries_actual_cost(&fake, &mut entries, decimal("6"), decimal("10"), REJECT)
        .await
        .expect("the resize");
    assert_eq!(entries[0].applied_adjustment, decimal("-4"));
    assert!(take_calls(&fake).reseeds.is_empty());
}
