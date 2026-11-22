use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::reservation_settlement::adjust::AdjustmentMode;
use crate::reservation_settlement::counters::SettlementCounters as _;
use crate::reservation_settlement::handle::stamp_reservation_actual_cost;
use crate::reservation_settlement::reconcile::{ReservationFinality, reconcile_budget_reservation};
use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::support::{
    K1, K2, decimal, entries_of, entry, handle, is_finalized, reserved, take_calls, value_of,
};

#[tokio::test]
async fn a_reservation_settles_from_its_estimate_to_the_actual_cost() {
    let (fake, reservation) = reserved(K1, "10");
    let pending = reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the settle");
    assert!(pending.is_empty());
    assert!(is_finalized(&reservation));
    let expected = vec![vec![(K1.to_owned(), decimal("-6"))]];
    assert_eq!(take_calls(&fake).pipelines, expected);
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        decimal("-6")
    );
    assert_eq!(value_of(&fake, K1), Some(decimal("4")));
}

#[tokio::test]
async fn a_deferred_settle_hands_the_deltas_back_and_keeps_the_record_open() {
    let (fake, reservation) = reserved(K1, "10");
    let pending = reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::LeaveOpen,
        AdjustmentMode::Defer,
    )
    .await
    .expect("the deferral");
    let expected = vec![PendingSpendIncrement {
        counter_key: K1.to_owned(),
        increment: decimal("-6"),
    }];
    assert_eq!(pending, expected);
    assert!(!is_finalized(&reservation));
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        Decimal::ZERO
    );
    assert_eq!(value_of(&fake, K1), Some(decimal("10")));
}

#[tokio::test]
async fn a_finalized_reservation_settles_only_once() {
    let (fake, reservation) = reserved(K1, "10");
    reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the settle");
    let again = reconcile_budget_reservation(
        &fake,
        &reservation,
        Decimal::ZERO,
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the second settle");
    assert!(again.is_empty());
    let calls = take_calls(&fake);
    assert_eq!(calls.pipelines.len(), 1);
    assert_eq!(calls.scopes.len(), 1);
}

#[tokio::test]
async fn a_failed_settle_leaves_the_reservation_open_for_a_retry() {
    let (mut fake, reservation) = reserved(K1, "10");
    fake.pipeline_failures.insert(K1.to_owned());
    reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect_err("the failed settle");
    assert!(!is_finalized(&reservation));
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        Decimal::ZERO
    );
    fake.pipeline_failures.clear();
    reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the retry");
    assert!(is_finalized(&reservation));
    assert_eq!(value_of(&fake, K1), Some(decimal("4")));
}

#[test]
fn stamping_records_the_target_delta_in_every_entry() {
    let mut unpriced = entry(K2, "0");
    unpriced.reserved_cost = None;
    let reservation = handle("10", vec![entry(K1, "10"), unpriced], "3");
    stamp_reservation_actual_cost(&reservation, decimal("4")).expect("the stamp");
    let stamped = entries_of(&reservation);
    assert_eq!(stamped[0].applied_adjustment, decimal("-6"));
    assert_eq!(stamped[1].applied_adjustment, decimal("-6"));
}

// LiteLLM checks that the entry dict carries a counter_key field, and our entries always carry one
#[test]
fn an_entry_without_a_counter_key_is_still_stamped() {
    let mut keyless = entry(K1, "10");
    keyless.counter_key = None;
    keyless.applied_adjustment = decimal("2");
    let reservation = handle("10", vec![keyless], "3");
    stamp_reservation_actual_cost(&reservation, decimal("4")).expect("the stamp");
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        decimal("-6")
    );
}

#[test]
fn stamping_still_records_on_a_finalized_reservation() {
    let reservation = handle("10", vec![entry(K1, "10")], "3");
    lock(&reservation.0).finalized = true;
    stamp_reservation_actual_cost(&reservation, decimal("4")).expect("the stamp");
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        decimal("-6")
    );
}

#[tokio::test]
async fn a_deferred_settle_and_a_stamp_record_the_same_target() {
    let (fake, reservation) = reserved(K1, "10");
    let pending = reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Defer,
    )
    .await
    .expect("the deferral");
    fake.increment_pipeline(&pending)
        .await
        .expect("the caller's pipeline");
    stamp_reservation_actual_cost(&reservation, decimal("4")).expect("the stamp");
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        decimal("-6")
    );
    assert_eq!(value_of(&fake, K1), Some(decimal("4")));
}

#[tokio::test]
async fn a_settle_after_a_deferred_stamp_applies_nothing_twice() {
    let (fake, reservation) = reserved(K1, "10");
    let pending = reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::LeaveOpen,
        AdjustmentMode::Defer,
    )
    .await
    .expect("the deferral");
    fake.increment_pipeline(&pending)
        .await
        .expect("the caller's pipeline");
    stamp_reservation_actual_cost(&reservation, decimal("4")).expect("the stamp");
    let again = reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the second settle");
    assert!(again.is_empty());
    assert_eq!(take_calls(&fake).pipelines.len(), 1);
    assert_eq!(value_of(&fake, K1), Some(decimal("4")));
}

#[tokio::test]
async fn a_counter_reseeded_before_the_settle_is_charged_the_actual_cost() {
    let (mut fake, reservation) = reserved(K1, "10");
    fake.values.lock().expect("the fake values").clear();
    fake.reseeded.insert(K1.to_owned());
    reconcile_budget_reservation(
        &fake,
        &reservation,
        decimal("4"),
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await
    .expect("the settle");
    let calls = take_calls(&fake);
    assert_eq!(calls.reseeds, vec![K1.to_owned()]);
    assert_eq!(calls.increments, vec![(K1.to_owned(), decimal("4"))]);
    assert_eq!(value_of(&fake, K1), Some(decimal("4")));
}
