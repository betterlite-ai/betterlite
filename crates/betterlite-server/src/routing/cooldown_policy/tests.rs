#![cfg(test)]

use std::sync::Arc;

use betterlite_router::cooldown_policy::event::CooldownObserver;

use super::CooldownObservers;

struct Marker;

impl CooldownObserver for Marker {}

#[test]
fn the_async_success_list_wins_over_the_global_one() {
    let success: Arc<dyn CooldownObserver> = Arc::new(Marker);
    let global: Arc<dyn CooldownObserver> = Arc::new(Marker);
    let observers = CooldownObservers::new(vec![Arc::clone(&success)], vec![Arc::clone(&global)]);
    assert!(Arc::ptr_eq(&observers.exporter().unwrap(), &success));
    assert_eq!(observers.handlers().len(), 1);
}

#[test]
fn the_global_list_answers_when_the_success_list_is_empty() {
    let global: Arc<dyn CooldownObserver> = Arc::new(Marker);
    let observers = CooldownObservers::new(Vec::new(), vec![Arc::clone(&global)]);
    assert!(Arc::ptr_eq(&observers.exporter().unwrap(), &global));
}

#[test]
fn no_callback_list_exports_no_cooldown() {
    let observers = CooldownObservers::new(Vec::new(), Vec::new());
    assert!(observers.exporter().is_none());
    assert!(observers.handlers().is_empty());
}
