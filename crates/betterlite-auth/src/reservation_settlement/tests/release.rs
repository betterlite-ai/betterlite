use std::sync::Arc;

use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::reservation_settlement::release::{
    invalidate_reservation_keys, release_budget_reservation, release_entries_best_effort,
    release_or_invalidate, release_reservation_on_cancel, release_unbound_reservation,
};

use super::support::{
    counters, decimal, entries_of, entry, handle, is_finalized, take_calls, value_of,
};

#[tokio::test]
async fn a_release_settles_the_reservation_to_zero() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    release_budget_reservation(&fake, &reservation)
        .await
        .expect("the release");
    assert!(is_finalized(&reservation));
    let expected = vec![vec![("spend:key:k1".to_owned(), decimal("-10"))]];
    assert_eq!(take_calls(&fake).pipelines, expected);
    assert_eq!(
        entries_of(&reservation)[0].applied_adjustment,
        decimal("-10")
    );
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(Decimal::ZERO));
}

#[tokio::test]
async fn a_partly_applied_entry_releases_only_the_remainder() {
    let fake = counters(&[("spend:key:k1", "8")]);
    let mut half = entry("spend:key:k1", "10");
    half.applied_adjustment = decimal("-2");
    let reservation = handle("10", vec![half], "3");
    release_budget_reservation(&fake, &reservation)
        .await
        .expect("the release");
    let expected = vec![vec![("spend:key:k1".to_owned(), decimal("-8"))]];
    assert_eq!(take_calls(&fake).pipelines, expected);
}

#[tokio::test]
async fn a_cancel_settles_at_the_input_cost() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    release_reservation_on_cancel(&fake, &reservation)
        .await
        .expect("the cancel release");
    assert!(is_finalized(&reservation));
    let expected = vec![vec![("spend:key:k1".to_owned(), decimal("-7"))]];
    assert_eq!(take_calls(&fake).pipelines, expected);
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("3")));
}

#[tokio::test]
async fn a_finalized_reservation_is_not_released_on_cancel() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    lock(&reservation.0).finalized = true;
    release_reservation_on_cancel(&fake, &reservation)
        .await
        .expect("the no-op");
    assert!(take_calls(&fake).pipelines.is_empty());
}

#[tokio::test]
async fn a_cancel_release_survives_the_request_task_being_dropped() {
    let fake = Arc::new(counters(&[("spend:key:k1", "10")]));
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    // the request never awaits the release: the server spawns it, so dropping the request cannot cancel it
    let tasks = TaskTracker::new();
    let spawned = Arc::clone(&fake);
    let settled = reservation.clone();
    let _join =
        tasks.spawn(async move { release_reservation_on_cancel(spawned.as_ref(), &settled).await });
    tasks.close();
    tasks.wait().await;
    assert!(is_finalized(&reservation));
    assert_eq!(take_calls(&fake).pipelines.len(), 1);
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(decimal("3")));
}

#[tokio::test]
async fn invalidation_drops_every_reserved_counter() {
    let fake = counters(&[("spend:key:k1", "10"), ("spend:key:k2", "10")]);
    let reservation = handle(
        "20",
        vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")],
        "0",
    );
    invalidate_reservation_keys(&fake, &reservation).await;
    let expected = vec!["spend:key:k1".to_owned(), "spend:key:k2".to_owned()];
    assert_eq!(take_calls(&fake).invalidations, expected);
    assert_eq!(value_of(&fake, "spend:key:k1"), None);
}

#[tokio::test]
async fn a_failed_release_invalidates_every_reserved_counter() {
    let mut fake = counters(&[("spend:key:k1", "10"), ("spend:key:k2", "10")]);
    fake.pipeline_failures.insert("spend:key:k1".to_owned());
    let reservation = handle(
        "20",
        vec![entry("spend:key:k1", "10"), entry("spend:key:k2", "10")],
        "3",
    );
    release_or_invalidate(&fake, &reservation).await;
    assert!(is_finalized(&reservation));
    let calls = take_calls(&fake);
    let expected = vec!["spend:key:k1".to_owned(), "spend:key:k2".to_owned()];
    assert_eq!(calls.invalidations, expected);
    assert!(calls.increments.is_empty());
}

#[tokio::test]
async fn a_released_reservation_is_not_released_twice() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    release_or_invalidate(&fake, &reservation).await;
    release_or_invalidate(&fake, &reservation).await;
    let calls = take_calls(&fake);
    assert_eq!(calls.pipelines.len(), 1);
    assert!(calls.invalidations.is_empty());
}

#[tokio::test]
async fn an_unbound_reservation_is_released_on_the_terminal_path() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    release_unbound_reservation(&fake, &reservation).await;
    assert!(is_finalized(&reservation));
    assert_eq!(take_calls(&fake).pipelines.len(), 1);
}

#[tokio::test]
async fn a_bound_reservation_is_left_for_its_callback() {
    let fake = counters(&[("spend:key:k1", "10")]);
    let reservation = handle("10", vec![entry("spend:key:k1", "10")], "3");
    lock(&reservation.0).callback_bound = true;
    release_unbound_reservation(&fake, &reservation).await;
    assert!(!is_finalized(&reservation));
    assert!(take_calls(&fake).pipelines.is_empty());
}

#[tokio::test]
async fn releasing_partial_entries_rolls_each_one_back_and_drops_a_failed_one() {
    let mut fake = counters(&[("spend:key:k1", "10"), ("spend:key:k2", "6")]);
    fake.pipeline_failures.insert("spend:key:k2".to_owned());
    let mut second = entry("spend:key:k2", "6");
    second.applied_adjustment = decimal("-4");
    let mut entries = vec![entry("spend:key:k1", "10"), second];
    release_entries_best_effort(&fake, &mut entries, decimal("10")).await;
    let calls = take_calls(&fake);
    let expected = vec![
        vec![("spend:key:k1".to_owned(), decimal("-10"))],
        vec![("spend:key:k2".to_owned(), decimal("-2"))],
    ];
    assert_eq!(calls.pipelines, expected);
    assert_eq!(calls.invalidations, vec!["spend:key:k2".to_owned()]);
    assert_eq!(entries[0].applied_adjustment, decimal("-10"));
    assert_eq!(value_of(&fake, "spend:key:k1"), Some(Decimal::ZERO));
    assert_eq!(value_of(&fake, "spend:key:k2"), None);
}
