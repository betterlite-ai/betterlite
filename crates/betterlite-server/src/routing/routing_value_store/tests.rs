#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_router::routing_value_store::{
    Clock, PinValue, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::local_store;

struct TestClock(Arc<Mutex<f64>>);

fn clock() -> TestClock {
    TestClock(Arc::new(Mutex::new(f64::default())))
}

fn at(clock: &TestClock, now: f64) {
    *clock.0.lock().unwrap() = now;
}

fn store(item_kib: i64, clock: &TestClock) -> RoutingValueStore {
    let source = Arc::clone(&clock.0);
    let clock: Clock = Arc::new(move || *source.lock().unwrap());
    local_store(
        &Settings {
            global_max_item_kib: item_kib,
            global_ttl: None,
        },
        clock,
    )
    .unwrap()
}

#[test]
fn local_store_uses_the_configured_item_limit() {
    let store = store(1, &clock());
    let small = RouteKey::parse("small").unwrap();
    store.set(&small, &RouteValue::Counter(1), None).unwrap();
    assert_eq!(
        store.get(&small).unwrap(),
        StoreRead::Value(RouteValue::Counter(1))
    );
    let large = RouteKey::parse("large").unwrap();
    let value = RouteValue::Pin(PinValue {
        model_id: "a".repeat(2048),
    });
    store.set(&large, &value, None).unwrap();
    assert_eq!(store.get(&large).unwrap(), StoreRead::Missing);
}

#[test]
fn local_store_keeps_values_under_a_mebibyte() {
    let store = store(1024, &clock());
    let key = RouteKey::parse("large").unwrap();
    let value = RouteValue::Pin(PinValue {
        model_id: "a".repeat(2048),
    });
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}

#[test]
fn local_store_reads_time_from_the_injected_clock() {
    let clock = clock();
    let store = store(1024, &clock);
    let key = RouteKey::parse("counter").unwrap();
    at(&clock, 0.0_f64);
    store
        .set(&key, &RouteValue::Counter(1), Some(10.0_f64))
        .unwrap();
    at(&clock, 10.0_f64);
    assert_eq!(
        store.get(&key).unwrap(),
        StoreRead::Value(RouteValue::Counter(1))
    );
    at(&clock, 10.5_f64);
    assert_eq!(store.get(&key).unwrap(), StoreRead::Missing);
}
