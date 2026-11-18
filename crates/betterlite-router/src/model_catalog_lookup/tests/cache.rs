use super::*;

#[test]
fn clear_all_metadata() {
    let slots = Slots::filled();
    let caches = slots.caches();
    caches.invalidate(Invalidation::ModelMetadata);
    assert!(slots.info.0.lock().unwrap().is_none());
    assert!(slots.deployment.0.lock().unwrap().is_none());
    assert!(slots.zero_cost.0.lock().unwrap().is_none());
    assert!(slots.rows.0.lock().unwrap().is_none());
    assert_eq!(
        slots.access.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    let mut catalog = catalog(&[priced("gpt-4o", "a", "1e-6")]);
    *slots.zero_cost.0.lock().unwrap() = Some(true);
    rebuild_ids(&mut catalog, vec![priced("gpt-4o", "a", "2e-6")], &caches).unwrap();
    assert!(slots.zero_cost.0.lock().unwrap().is_none());
}

#[test]
fn clear_access_only() {
    let slots = Slots::filled();
    slots.caches().invalidate(Invalidation::AccessGroups);
    assert!(slots.access.0.lock().unwrap().is_none());
    assert_eq!(
        slots.info.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    assert_eq!(
        slots.deployment.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    assert_eq!(slots.zero_cost.0.lock().unwrap().clone(), Some(true));
    assert_eq!(
        slots.rows.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
}
