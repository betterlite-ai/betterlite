use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tokio::time::timeout;

use crate::spend_reseed::locks::LockRegistry;

const WAIT: Duration = Duration::from_secs(1);
const PAUSE: Duration = Duration::from_millis(10);

#[test]
fn a_hit_moves_to_the_end_and_overflow_evicts_the_oldest() {
    let registry = LockRegistry::from(2);
    let first = registry.operation("a");
    let second = registry.operation("b");
    let again = registry.operation("a");
    assert!(Arc::ptr_eq(&first, &again));
    assert!(!Arc::ptr_eq(&first, &second));

    // the hit made "b" the oldest entry, so inserting "c" evicts it and rebuilds on the next call
    let third = registry.operation("c");
    let rebuilt = registry.operation("b");
    assert!(!Arc::ptr_eq(&second, &rebuilt));
    assert!(Arc::ptr_eq(&third, &registry.operation("c")));

    // a bound of one drops the oldest entry, not the one just inserted
    let tight = LockRegistry::from(1);
    let older = tight.operation("x");
    let newer = tight.operation("y");
    assert!(Arc::ptr_eq(&newer, &tight.operation("y")));
    assert!(!Arc::ptr_eq(&older, &tight.operation("x")));
}

#[tokio::test(start_paused = true)]
async fn an_evicted_held_operation_no_longer_serializes_the_key() {
    let registry = LockRegistry::from(1);
    let guard = registry.hold("held").await;
    // the overflow evicts the running operation, so a new one starts with no promise of singleflight
    let _other = registry.operation("other");
    let second = timeout(WAIT, registry.hold("held"))
        .await
        .expect("the evicted key acquires without waiting");
    drop(second);
    drop(guard);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-3)]
#[tokio::test(start_paused = true)]
async fn a_nonpositive_bound_strips_every_entry(#[case] bound: i64) {
    let registry = LockRegistry::from(bound);
    let first = registry.operation("z");
    let second = registry.operation("z");
    assert!(!Arc::ptr_eq(&first, &second));

    let held = registry.hold("z").await;
    let other = timeout(WAIT, registry.hold("z"))
        .await
        .expect("a stripped entry never serializes");
    drop(other);
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn an_owner_serializes_the_key_and_releasing_wakes_the_next_waiter() {
    let registry = LockRegistry::from(10);
    let first = registry.hold("k").await;
    let second = async {
        let _guard = registry.hold("k").await;
        true
    };
    tokio::pin!(second);
    assert!(
        timeout(PAUSE, second.as_mut()).await.is_err(),
        "the second holder must park while the first owns the key"
    );
    drop(first);
    assert!(
        timeout(WAIT, second)
            .await
            .expect("the release wakes the waiter")
    );
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_waiter_leaves_the_key_usable() {
    let registry = LockRegistry::from(10);
    let first = registry.hold("k").await;
    let mut waiter = Box::pin(async {
        let _guard = registry.hold("k").await;
    });
    assert!(timeout(PAUSE, waiter.as_mut()).await.is_err());
    // dropping a parked waiter is cancellation, and it must not leave the operation running
    drop(waiter);
    drop(first);
    let reacquired = timeout(WAIT, registry.hold("k"))
        .await
        .expect("the key is free after cancellation");
    drop(reacquired);
}

#[tokio::test]
async fn dropping_the_guard_releases_the_key() {
    let registry = LockRegistry::from(10);
    let guard = registry.hold("k").await;
    drop(guard);
    let again = timeout(WAIT, registry.hold("k"))
        .await
        .expect("a dropped guard releases the key");
    drop(again);
}
