use std::time::Duration;

use tokio::time::timeout;

use rust_decimal::Decimal;

use super::super::FloatIncrement;
use super::super::error::{StateStoreError, StoreWarning};
use super::fake_redis::FakeRedis;
use super::support::{TestClock, breaker, route_key, same_f64, store};
use crate::routing_value_store::{
    MAX_VALUE_BYTES, PinValue, RouteIncrement, RouteValue, StoreError, StoreRead,
};

pub(crate) async fn run(name: &str) {
    if ttl_and_size_cases(name).await {
        return;
    }
    if decode_and_overflow_cases(name).await {
        return;
    }
    if expiry_and_float_cases(name).await {
        return;
    }
    panic!("unknown case {name}");
}

async fn ttl_and_size_cases(name: &str) -> bool {
    match name {
        "ttl_zero_is_rejected" => ttl_is_rejected(0.0_f64).await,
        "ttl_negative_is_rejected" => ttl_is_rejected(-1.0_f64).await,
        "ttl_infinite_is_rejected" => ttl_is_rejected(f64::INFINITY).await,
        "ttl_nan_is_rejected" => ttl_nan_is_rejected().await,
        "fractional_ttl_truncates" => fractional_ttl_truncates().await,
        "oversized_set_is_not_written" => oversized_set_is_not_written().await,
        "oversized_compare_and_set_reports_the_current" => {
            oversized_compare_and_set_reports_the_current().await;
        }
        _ => return false,
    }
    true
}

async fn decode_and_overflow_cases(name: &str) -> bool {
    match name {
        "invalid_stored_value_reads_as_missing" => invalid_stored_value_reads_as_missing().await,
        "invalid_stored_value_fails_an_increment" => {
            invalid_stored_value_fails_an_increment().await;
        }
        "batch_decode_failure_misses_every_key" => batch_decode_failure_misses_every_key().await,
        "counter_overflow_is_reported" => counter_overflow_is_reported().await,
        "spend_overflow_is_reported" => spend_overflow_is_reported().await,
        "increment_mismatched_kind_is_not_incrementable" => {
            increment_mismatched_kind_is_not_incrementable().await;
        }
        _ => return false,
    }
    true
}

async fn expiry_and_float_cases(name: &str) -> bool {
    match name {
        "increment_keeps_a_live_expiry" => increment_keeps_a_live_expiry().await,
        "increment_adds_expiry_when_none" => increment_adds_expiry_when_none().await,
        "increment_without_ttl_keeps_no_expiry" => increment_without_ttl_keeps_no_expiry().await,
        "set_resets_a_live_expiry" => set_resets_a_live_expiry().await,
        "namespace_is_not_applied_twice" => namespace_is_not_applied_twice().await,
        "get_ttl_maps_absent_and_unexpiring_to_none" => {
            get_ttl_maps_absent_and_unexpiring_to_none().await;
        }
        "float_increment_invalid_ttl_uses_the_default" => {
            float_increment_invalid_ttl_uses_the_default().await;
        }
        "float_pipeline_invalid_ttl_is_an_error" => float_pipeline_invalid_ttl_is_an_error().await,
        "float_pipeline_zero_ttl_is_sent" => float_pipeline_zero_ttl_is_sent().await,
        "cancelled_compare_and_set_leaves_the_connection_usable" => {
            cancelled_compare_and_set_leaves_the_connection_usable().await;
        }
        _ => return false,
    }
    true
}

async fn ttl_is_rejected(seconds: f64) {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    let error = store
        .set(&key, &RouteValue::Counter(1), Some(seconds))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::InvalidTtl { seconds: reported } if same_f64(reported, seconds)
    ));
    assert!(server.raw("counter").is_none());
}

async fn ttl_nan_is_rejected() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let error = store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
            Some(f64::NAN),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::InvalidTtl { seconds } if seconds.is_nan()
    ));
}

async fn fractional_ttl_truncates() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    store
        .set(
            &route_key("counter"),
            &RouteValue::Counter(1),
            Some(1.9_f64),
        )
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(1));
}

fn oversized_pin() -> RouteValue {
    RouteValue::Pin(PinValue {
        model_id: "a".repeat(MAX_VALUE_BYTES),
    })
}

async fn oversized_set_is_not_written() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let observed = store
        .set(&route_key("pin"), &oversized_pin(), None)
        .await
        .unwrap();
    assert!(observed.warning.is_none());
    assert!(server.raw("pin").is_none());
}

async fn oversized_compare_and_set_reports_the_current() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("pin");
    store
        .set(&key, &RouteValue::Counter(1), None)
        .await
        .unwrap();
    let observed = store
        .compare_and_set(
            &key,
            &StoreRead::Value(RouteValue::Counter(1)),
            &oversized_pin(),
            None,
        )
        .await
        .unwrap();
    assert!(!observed.value.swapped);
    assert_eq!(
        observed.value.current,
        StoreRead::Value(RouteValue::Counter(1))
    );
    assert!(server.raw("pin").is_some());
}

async fn invalid_stored_value_reads_as_missing() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.seed("counter", b"not-json", -1);
    let observed = store.get(&route_key("counter")).await.unwrap();
    assert_eq!(observed.value, StoreRead::Missing);
    assert!(matches!(
        observed.warning,
        Some(StoreWarning::Unavailable {
            operation: "async_get_cache",
            ..
        })
    ));
}

async fn invalid_stored_value_fails_an_increment() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.seed("counter", b"not-json", -1);
    let error = store
        .increment(&route_key("counter"), &RouteIncrement::Counter(1), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::Store(StoreError::InvalidValue { .. })
    ));
    assert_eq!(
        server.raw("counter").map(|entry| entry.0),
        Some(b"not-json".to_vec())
    );
}

async fn batch_decode_failure_misses_every_key() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let valid = serde_json::to_vec(&RouteValue::Counter(3)).unwrap();
    server.seed("a", &valid, -1);
    server.seed("b", b"not-json", -1);
    let observed = store
        .batch_get(&[route_key("a"), route_key("b")])
        .await
        .unwrap();
    assert_eq!(observed.value, vec![StoreRead::Missing, StoreRead::Missing]);
    assert!(matches!(
        observed.warning,
        Some(StoreWarning::Unavailable {
            operation: "async_batch_get_cache",
            ..
        })
    ));
}

async fn counter_overflow_is_reported() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("overflow");
    store
        .set(&key, &RouteValue::Counter(i64::MAX), None)
        .await
        .unwrap();
    let error = store
        .increment(&key, &RouteIncrement::Counter(1), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::Store(StoreError::CounterOverflow { key: reported })
            if reported == "overflow"
    ));
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Counter(i64::MAX))
    );
}

async fn spend_overflow_is_reported() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("overflow");
    store
        .set(&key, &RouteValue::Spend(Decimal::MAX), None)
        .await
        .unwrap();
    let error = store
        .increment(&key, &RouteIncrement::Spend(Decimal::ONE), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::Store(StoreError::SpendOverflow { key: reported })
            if reported == "overflow"
    ));
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Spend(Decimal::MAX))
    );
}

async fn increment_mismatched_kind_is_not_incrementable() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("pin");
    store
        .set(
            &key,
            &RouteValue::Pin(PinValue {
                model_id: "model".to_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    let error = store
        .increment(&key, &RouteIncrement::Counter(1), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::Store(StoreError::NotIncrementable { key: reported })
            if reported == "pin"
    ));
}

async fn increment_keeps_a_live_expiry() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), Some(30.0_f64))
        .await
        .unwrap();
    store
        .increment(&key, &RouteIncrement::Counter(1), Some(100.0_f64))
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(30));
}

async fn increment_adds_expiry_when_none() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), None)
        .await
        .unwrap();
    store
        .increment(&key, &RouteIncrement::Counter(1), Some(100.0_f64))
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(100));
}

async fn increment_without_ttl_keeps_no_expiry() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), None)
        .await
        .unwrap();
    store
        .increment(&key, &RouteIncrement::Counter(1), None)
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(-1));
}

async fn set_resets_a_live_expiry() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), Some(30.0_f64))
        .await
        .unwrap();
    store
        .set(&key, &RouteValue::Counter(2), Some(5.0_f64))
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(5));
}

async fn namespace_is_not_applied_twice() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, Some("router"), breaker(5, 60_000), &clock);
    store
        .set(&route_key("router:counter"), &RouteValue::Counter(1), None)
        .await
        .unwrap();
    assert!(server.raw("router:counter").is_some());
    assert!(server.raw("router:router:counter").is_none());
}

async fn get_ttl_maps_absent_and_unexpiring_to_none() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.seed("unexpiring", b"1", -1);
    server.seed("expiring", b"1", 0);
    assert_eq!(
        store.get_ttl(&route_key("unexpiring")).await.unwrap().value,
        None
    );
    assert_eq!(
        store.get_ttl(&route_key("absent")).await.unwrap().value,
        None
    );
    assert_eq!(
        store.get_ttl(&route_key("expiring")).await.unwrap().value,
        Some(0)
    );
}

async fn float_increment_invalid_ttl_uses_the_default() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let observed = store
        .increment_float(&route_key("counter"), 1.0_f64, Some(f64::NAN), false)
        .await
        .unwrap();
    assert!(same_f64(observed.value, 1.0_f64));
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(60));
}

async fn float_pipeline_invalid_ttl_is_an_error() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let entries = [FloatIncrement {
        key: route_key("counter"),
        value: 1.0_f64,
        ttl_seconds: Some(f64::INFINITY),
    }];
    let error = store.increment_float_pipeline(&entries).await.unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::InvalidTtl { seconds } if seconds.is_infinite()
    ));
}

async fn float_pipeline_zero_ttl_is_sent() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let entries = [FloatIncrement {
        key: route_key("counter"),
        value: 2.0_f64,
        ttl_seconds: Some(0.0_f64),
    }];
    let values = store
        .increment_float_pipeline(&entries)
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(values.len(), 1);
    assert!(same_f64(values[0], 2.0_f64));
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(0));
}

async fn cancelled_compare_and_set_leaves_the_connection_usable() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("pin");
    let value = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let cancelled = timeout(
        Duration::ZERO,
        store.compare_and_set(&key, &StoreRead::Missing, &value, None),
    )
    .await;
    let _cancelled = cancelled.unwrap_err();
    let retried = store
        .compare_and_set(&key, &StoreRead::Missing, &value, None)
        .await
        .unwrap();
    assert!(retried.value.swapped);
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(value)
    );
}
