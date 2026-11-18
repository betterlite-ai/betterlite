use std::sync::Arc;

use crate::spend_reseed::locks::LockRegistry;

#[tokio::test]
async fn lru_hit_overflow_zero_negative_and_held_eviction() {
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
    assert!(held.try_lock().is_err());
    assert!(replaced.try_lock().is_ok());
    drop(guard);

    // a zero or negative bound strips every entry it inserts, so callers stop sharing a lock
    for bound in [0_i64, -3] {
        let registry = LockRegistry::from(bound);
        let first = registry.lock_for("z");
        let second = registry.lock_for("z");
        assert!(!Arc::ptr_eq(&first, &second));
    }
}
