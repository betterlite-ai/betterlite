#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_router::routing_value_store::{
    PinValue, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::local_store;

struct TestClock(Arc<Mutex<f64>>);

fn clock() -> TestClock {
    TestClock(Arc::new(Mutex::new(f64::default())))
}

fn at(clock: &TestClock, now: f64) {
    *clock.0.lock().unwrap() = now;
}

fn store(item_kib: i64, test_clock: &TestClock) -> RoutingValueStore {
    let source = Arc::clone(&test_clock.0);
    local_store(
        &Settings {
            global_max_item_kib: item_kib,
            global_ttl: None,
        },
        Arc::new(move || *source.lock().unwrap()),
    )
    .unwrap()
}

#[test]
fn local_store_uses_the_in_memory_constructor_item_default() {
    // router.py builds InMemoryCache() with no arguments, so the 1024 KiB default holds whatever the env says
    let store = store(1, &clock());
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
