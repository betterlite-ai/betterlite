use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use tokio::time::timeout;

use super::super::RoutingStateStore;
use super::super::error::{StateStoreError, StoreWarning};
use super::support::{TestClock, breaker, route_key, same_f64, seed, settings, store};
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::{
    MAX_VALUE_BYTES, PinValue, RouteIncrement, RouteKey, RouteValue, StoreError, StoreRead,
};

pub(crate) async fn run(name: &str) {
    if read_cases(name).await {
        return;
    }
    if write_cases(name).await {
        return;
    }
    if expiry_cases(name).await {
        return;
    }
    if pipeline_cases(name).await {
        return;
    }
    match name {
        "increment_succeeds_under_contention" => increment_succeeds_under_contention().await,
        "increment_keeps_an_expiring_key_at_ttl_zero" => {
            increment_keeps_an_expiring_key_at_ttl_zero().await;
        }
        "increment_conflicts_after_the_update_window" => {
            increment_conflicts_after_the_update_window().await;
        }
        _ => super::server::run(name).await,
    }
}

#[rustfmt::skip]
async fn read_cases(name: &str) -> bool {
    match name {
        "ttl_zero_is_rejected" => ttl_is_rejected(0.0_f64).await,
        "ttl_negative_is_rejected" => ttl_is_rejected(-1.0_f64).await,
        "ttl_infinite_is_rejected" => ttl_is_rejected(f64::INFINITY).await,
        "ttl_nan_is_rejected" => ttl_is_rejected(f64::NAN).await,
        "fractional_ttl_truncates" => fractional_ttl_truncates().await,
        "oversized_compare_and_set_does_not_write" => oversized_compare_and_set_does_not_write().await,
        "route_key_length_boundary" => route_key_length_boundary().await,
        "invalid_stored_value_reads_as_missing" => invalid_stored_value_reads_as_missing().await,
        "invalid_stored_value_fails_an_increment" => invalid_stored_value_fails_an_increment().await,
        "batch_decode_failure_misses_every_key" => batch_decode_failure_misses_every_key().await,
        _ => return false,
    }
    true
}

async fn write_cases(name: &str) -> bool {
    if name == "counter_overflow_is_reported" {
        overflow_is_reported(
            RouteValue::Counter(i64::MAX),
            RouteIncrement::Counter(1),
            by_counter,
        )
        .await;
        return true;
    }
    if name == "spend_overflow_is_reported" {
        overflow_is_reported(
            RouteValue::Spend(Decimal::MAX),
            RouteIncrement::Spend(Decimal::ONE),
            by_spend,
        )
        .await;
        return true;
    }
    if name == "increment_mismatched_kind_is_not_incrementable" {
        increment_mismatched_kind_is_not_incrementable().await;
        return true;
    }
    if name == "increment_keeps_a_live_expiry" {
        increment_ttl_case(30, Some(100.0_f64), 30).await;
        return true;
    }
    if name == "increment_adds_expiry_when_none" {
        increment_ttl_case(-1, Some(100.0_f64), 100).await;
        return true;
    }
    if name == "increment_without_ttl_uses_the_default" {
        increment_ttl_case(-1, None, 60).await;
        return true;
    }
    false
}

#[rustfmt::skip]
async fn expiry_cases(name: &str) -> bool {
    if name == "get_ttl_maps_absent_and_unexpiring_to_none" { get_ttl_maps_absent_and_unexpiring_to_none().await; return true; }
    if name == "float_increment_invalid_ttl_uses_the_default" { float_increment_invalid_ttl_uses_the_default().await; return true; }
    false
}

#[rustfmt::skip]
async fn pipeline_cases(name: &str) -> bool {
    if name == "float_pipeline_invalid_ttl_is_an_error" { float_pipeline_invalid_ttl_is_an_error().await; return true; }
    if name == "float_pipeline_zero_ttl_is_sent" { float_pipeline_zero_ttl_is_sent().await; return true; }
    if name == "cancelled_compare_and_set_leaves_the_connection_usable" { cancelled_compare_and_set_leaves_the_connection_usable().await; return true; }
    false
}

async fn ttl_is_rejected(seconds: f64) {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let error = store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
            Some(seconds),
        )
        .await
        .unwrap_err();
    let rejected = matches!(
        error,
        StateStoreError::InvalidTtl { seconds: reported }
            if same_f64(reported, seconds) || (reported.is_nan() && seconds.is_nan())
    );
    assert!(rejected);
}

async fn increment_keeps_an_expiring_key_at_ttl_zero() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "counter", &RouteValue::Counter(5), 0);
    let observed = store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
            Some(100.0_f64),
        )
        .await
        .unwrap();
    assert_eq!(observed.value, RouteValue::Counter(6));
    let (bytes, ttl) = server.raw("counter").unwrap();
    assert_eq!(
        serde_json::from_slice::<RouteValue>(&bytes).unwrap(),
        RouteValue::Counter(6)
    );
    assert_eq!(ttl, 0);
}

async fn fractional_ttl_truncates() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
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

async fn oversized_compare_and_set_does_not_write() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("pin");
    let observed = store
        .compare_and_set(&key, &StoreRead::Missing, &oversized_pin(), None)
        .await
        .unwrap();
    assert!(!observed.value.swapped);
    assert_eq!(observed.value.current, StoreRead::Missing);
    assert!(server.raw("pin").is_none());
}

async fn route_key_length_boundary() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let raw = "k".repeat(1_024);
    let key = route_key(&raw);
    store
        .increment(&key, &RouteIncrement::Counter(1), None)
        .await
        .unwrap();
    assert!(server.raw(&raw).is_some());
    assert!(matches!(
        RouteKey::parse(&"k".repeat(1_025)),
        Err(StoreError::KeyTooLong {
            bytes: 1_025,
            limit: 1_024
        })
    ));
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
    seed(&server, "a", &RouteValue::Counter(3), -1);
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

fn by_counter(error: &StateStoreError) -> bool {
    matches!(
        error,
        StateStoreError::Store(StoreError::CounterOverflow { key }) if key.as_str() == "overflow"
    )
}

fn by_spend(error: &StateStoreError) -> bool {
    matches!(
        error,
        StateStoreError::Store(StoreError::SpendOverflow { key }) if key.as_str() == "overflow"
    )
}

async fn overflow_is_reported(
    seeded: RouteValue,
    by: RouteIncrement,
    check: fn(&StateStoreError) -> bool,
) {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "overflow", &seeded, -1);
    let error = store
        .increment(&route_key("overflow"), &by, None)
        .await
        .unwrap_err();
    assert!(check(&error));
    assert_eq!(
        server.raw("overflow").map(|entry| entry.0),
        Some(serde_json::to_vec(&seeded).unwrap())
    );
}

async fn increment_mismatched_kind_is_not_incrementable() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(
        &server,
        "pin",
        &RouteValue::Pin(PinValue {
            model_id: "model".to_owned(),
        }),
        -1,
    );
    let error = store
        .increment(&route_key("pin"), &RouteIncrement::Counter(1), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StateStoreError::Store(StoreError::NotIncrementable { key: reported })
            if reported == "pin"
    ));
}

async fn increment_ttl_case(seeded: i64, requested: Option<f64>, expected: i64) {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "counter", &RouteValue::Counter(1), seeded);
    store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
            requested,
        )
        .await
        .unwrap();
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(expected));
}

async fn get_ttl_maps_absent_and_unexpiring_to_none() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "unexpiring", &RouteValue::Counter(1), -1);
    seed(&server, "expiring", &RouteValue::Counter(1), 0);
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
    let entries = [(route_key("counter"), 1.0_f64, Some(f64::INFINITY))];
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
    let entries = [(route_key("counter"), 2.0_f64, Some(0.0_f64))];
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

async fn increment_succeeds_under_contention() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .increment(&key, &RouteIncrement::Counter(2), None)
        .await
        .unwrap();
    server.rival_writes_before_exec(4);
    let observed = store
        .increment(&key, &RouteIncrement::Counter(3), None)
        .await
        .unwrap();
    assert_eq!(observed.value, RouteValue::Counter(9));
    let stored: RouteValue = serde_json::from_slice(&server.raw("counter").unwrap().0).unwrap();
    assert_eq!(stored, RouteValue::Counter(9));
}

async fn increment_conflicts_after_the_update_window() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let mut section = settings(&server, None, breaker(5, 60_000));
    section.redis.socket_timeout = 0.3_f64;
    let store = RoutingStateStore::new(&section, clock.breaker_clock()).unwrap();
    server.rival_writes_before_exec(u64::MAX);
    let started = Instant::now();
    let error = store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1_000_000_000),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, StateStoreError::Conflict { .. }),
        "{error:?}"
    );
    assert!(started.elapsed() >= Duration::from_millis(300));
    let (bytes, _ttl) = server.raw("counter").unwrap();
    let stored: RouteValue = serde_json::from_slice(&bytes).unwrap();
    assert!(
        matches!(stored, RouteValue::Counter(count) if count < 1_000_000_000),
        "{stored:?}"
    );
}
