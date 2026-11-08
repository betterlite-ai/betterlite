#![cfg(test)]

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_router::routing_value_store::{PinValue, RouteKey, RouteValue, StoreRead};

use super::local_store;

fn section(item_kib: i64) -> Settings {
    Settings {
        global_max_item_kib: item_kib,
        global_ttl: None,
    }
}

#[test]
fn local_store_uses_the_configured_item_limit() {
    let store = local_store(&section(1)).unwrap();
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
    let store = local_store(&Settings::default()).unwrap();
    let key = RouteKey::parse("large").unwrap();
    let value = RouteValue::Pin(PinValue {
        model_id: "a".repeat(2048),
    });
    store.set(&key, &value, None).unwrap();
    assert_eq!(store.get(&key).unwrap(), StoreRead::Value(value));
}
