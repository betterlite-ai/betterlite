use std::sync::Arc;

use rstest::rstest;

use crate::spend_reseed::locks::LockRegistry;

#[rstest]
#[case::zero(0_i64)]
#[case::negative(-3)]
#[tokio::test]
async fn lru_hit_overflow_zero_negative_and_held_eviction(#[case] bound: i64) {
    let registry = LockRegistry::from(2);
    let first = registry.lock_for("a");
    let second = registry.lock_for("b");
    let again = registry.lock_for("a");
    assert!(Arc::ptr_eq(&first, &again));
    assert!(!Arc::ptr_eq(&first, &second));

    // "b" is the oldest entry now, so inserting "c" evicts it and a new request rebuilds it
    let third = registry.lock_for("c");
    let rebuilt = registry.lock_for("b");
    assert!(!Arc::ptr_eq(&second, &rebuilt));
    assert!(Arc::ptr_eq(&third, &registry.lock_for("c")));

    // a held lock is evicted like any other when the bound overflows
    let held_registry = LockRegistry::from(1);
    let held = held_registry.lock_for("held");
    let guard = held.lock().await;
    let _overflow = held_registry.lock_for("other");
    let replaced = held_registry.lock_for("held");
    assert!(!Arc::ptr_eq(&held, &replaced));
    held.try_lock().unwrap_err();
    replaced.try_lock().unwrap();
    drop(guard);

    // a zero or negative bound strips every entry it inserts, so callers stop sharing a lock
    let bounded = LockRegistry::from(bound);
    let zero_first = bounded.lock_for("z");
    let zero_second = bounded.lock_for("z");
    assert!(!Arc::ptr_eq(&zero_first, &zero_second));
}
