#![cfg(test)]

use std::sync::Arc;

use betterlite_router::cooldown_policy::event::CooldownObserver;

use super::CooldownObservers;

struct Marker;

impl CooldownObserver for Marker {}

#[test]
fn the_exporter_comes_from_the_success_list_then_the_global_one() {
    let success: Arc<dyn CooldownObserver> = Arc::new(Marker);
    let global: Arc<dyn CooldownObserver> = Arc::new(Marker);
    let both = CooldownObservers {
        success: vec![Arc::clone(&success)],
        global: vec![Arc::clone(&global)],
    };
    assert!(Arc::ptr_eq(&both.exporter().unwrap(), &success));
    let global_only = CooldownObservers {
        success: Vec::new(),
        global: vec![Arc::clone(&global)],
    };
    assert!(Arc::ptr_eq(&global_only.exporter().unwrap(), &global));
    let none = CooldownObservers {
        success: Vec::new(),
        global: Vec::new(),
    };
    assert!(none.exporter().is_none());
}
