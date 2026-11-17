#![cfg(test)]

use std::sync::Arc;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_router::fallback_observations::observer::FallbackObserver;

use super::FallbackObservations;

struct NoopObserver;

impl FallbackObserver for NoopObserver {}

fn observations() -> FallbackObservations {
    FallbackObservations {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        observers: Vec::new(),
    }
}

#[test]
fn the_same_observer_registered_twice_is_dispatched_once() {
    let mut registry = observations();
    let observer: Arc<dyn FallbackObserver> = Arc::new(NoopObserver);
    registry.register_observer(Arc::clone(&observer));
    registry.register_observer(observer);
    assert_eq!(registry.observers.len(), 1);
}

#[test]
fn distinct_observers_are_kept_in_registration_order() {
    let mut registry = observations();
    let first: Arc<dyn FallbackObserver> = Arc::new(NoopObserver);
    let second: Arc<dyn FallbackObserver> = Arc::new(NoopObserver);
    registry.register_observer(Arc::clone(&first));
    registry.register_observer(Arc::clone(&second));
    assert_eq!(registry.observers.len(), 2);
    assert!(Arc::ptr_eq(&registry.observers[0], &first));
    assert!(Arc::ptr_eq(&registry.observers[1], &second));
}
