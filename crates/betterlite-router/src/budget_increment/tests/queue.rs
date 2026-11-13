use rust_decimal::Decimal;

use super::support::queued;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::queue::IncrementQueue;

#[test]
fn detach_returns_the_queued_increments_and_keeps_them() {
    let mut queue = IncrementQueue::default();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(10, 2), 30));
    assert_eq!(
        queue.detach(),
        vec![queued("spend:a", Decimal::new(10, 2), 30)]
    );
    assert_eq!(
        queue.detach(),
        vec![queued("spend:a", Decimal::new(10, 2), 30)]
    );
}

#[test]
fn a_second_detach_returns_the_reserved_batch_while_new_increments_queue() {
    let mut queue = IncrementQueue::default();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(10, 2), 30));
    let reserved = queue.detach();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(20, 2), 20));
    assert_eq!(queue.detach(), reserved);
}

#[test]
fn clear_detached_releases_the_reserved_batch() {
    let mut queue = IncrementQueue::default();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(10, 2), 30));
    queue.detach();
    queue.detached = None;
    assert!(queue.detach().is_empty());
}

#[test]
fn an_empty_queue_detaches_nothing() {
    let mut queue = IncrementQueue::default();
    assert!(queue.detach().is_empty());
}

#[test]
fn requeue_merges_by_key_and_sums_the_costs() {
    let mut queue = IncrementQueue::default();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(10, 2), 30));
    queue
        .queued
        .push(queued("spend:b", Decimal::new(50, 1), 10));
    queue.detach();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(20, 2), 20));
    queue.queued.push(queued("spend:c", Decimal::ONE, 5));
    queue.requeue().unwrap();
    assert_eq!(
        queue.detach(),
        vec![
            queued("spend:a", Decimal::new(30, 2), 20),
            queued("spend:b", Decimal::new(50, 1), 10),
            queued("spend:c", Decimal::ONE, 5),
        ]
    );
}

#[test]
fn requeue_without_a_detached_batch_changes_nothing() {
    let mut queue = IncrementQueue::default();
    queue
        .queued
        .push(queued("spend:a", Decimal::new(10, 2), 30));
    queue.requeue().unwrap();
    assert_eq!(
        queue.detach(),
        vec![queued("spend:a", Decimal::new(10, 2), 30)]
    );
}

#[test]
fn a_requeue_overflow_keeps_the_detached_batch() {
    let mut queue = IncrementQueue::default();
    queue.queued.push(queued("spend:a", Decimal::MAX, 30));
    let reserved = queue.detach();
    queue.queued.push(queued("spend:a", Decimal::ONE, 20));
    let error = queue.requeue().unwrap_err();
    assert!(matches!(error, IncrementError::SpendOverflow { key } if key == "spend:a"));
    assert_eq!(queue.detach(), reserved);
}
