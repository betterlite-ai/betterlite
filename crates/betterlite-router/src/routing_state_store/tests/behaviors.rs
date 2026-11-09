use std::collections::BTreeSet;

use rust_decimal::Decimal;
use secrecy::SecretString;

use super::super::error::{StateStoreError, StoreWarning};
use super::super::{FloatIncrement, RedisSettings};
use super::fake_redis::FakeRedis;
use super::support::{
    TestClock, breaker, local_only_store, redis_settings, route_key, same_f64, secret, store,
};
use crate::redis_failure_policy::BreakerEvent;
use crate::routing_value_store::{
    CompareOutcome, PinValue, RouteIncrement, RouteValue, SchedulerTuple, StoreRead,
};

pub(crate) async fn run(name: &str) {
    if read_and_write_cases(name).await {
        return;
    }
    if failure_cases(name).await {
        return;
    }
    panic!("unknown case {name}");
}

async fn read_and_write_cases(name: &str) -> bool {
    match name {
        "typed_values_round_trip" => typed_values_round_trip().await,
        "namespace_prefixes_every_key" => namespace_prefixes_every_key().await,
        "batch_get_keeps_order_and_duplicates" => batch_get_keeps_order_and_duplicates().await,
        "increment_counter_from_missing_and_existing" => {
            increment_counter_from_missing_and_existing().await;
        }
        "increment_spend_is_exact" => increment_spend_is_exact().await,
        "float_increments_and_pipeline" => float_increments_and_pipeline().await,
        "empty_float_pipeline_is_none" => empty_float_pipeline_is_none().await,
        "compare_and_set_first_writer" => compare_and_set_first_writer().await,
        "compare_and_set_matching_value_swaps" => compare_and_set_matching_value_swaps().await,
        "set_add_accumulates_members" => set_add_accumulates_members().await,
        "delete_removes_value" => delete_removes_value().await,
        "ttl_read_and_refresh" => ttl_read_and_refresh().await,
        _ => return false,
    }
    true
}

async fn failure_cases(name: &str) -> bool {
    match name {
        "transport_failure_falls_back_for_reads" => transport_failure_falls_back_for_reads().await,
        "transport_failure_fails_increments" => transport_failure_fails_increments().await,
        "breaker_rejects_after_a_swallowed_failure" => {
            breaker_rejects_after_a_swallowed_failure().await;
        }
        "breaker_recovers_after_the_timeout" => breaker_recovers_after_the_timeout().await,
        "attach_redis_only_when_none_is_set" => attach_redis_only_when_none_is_set().await,
        "local_backend_round_trips" => local_backend_round_trips().await,
        "credentials_stay_out_of_errors" => credentials_stay_out_of_errors().await,
        _ => return false,
    }
    true
}

async fn typed_values_round_trip() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let values = [
        RouteValue::Counter(7),
        RouteValue::Spend(Decimal::new(1225, 2)),
        RouteValue::Pin(PinValue {
            model_id: "model".to_owned(),
        }),
        RouteValue::SchedulerTuple(SchedulerTuple {
            priority: 2,
            request_id: "request".to_owned(),
        }),
        RouteValue::Members(BTreeSet::from(["a".to_owned(), "b".to_owned()])),
    ];
    for (index, value) in values.iter().enumerate() {
        let key = route_key(&format!("value-{index}"));
        store.set(&key, value, None).await.unwrap();
        let observed = store.get(&key).await.unwrap();
        assert_eq!(observed.value, StoreRead::Value(value.clone()));
        assert!(observed.warning.is_none());
    }
}

async fn namespace_prefixes_every_key() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, Some("router"), breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), None)
        .await
        .unwrap();
    assert!(server.raw("router:counter").is_some());
    assert!(server.raw("counter").is_none());
    let observed = store.get(&key).await.unwrap();
    assert_eq!(observed.value, StoreRead::Value(RouteValue::Counter(1)));
}

async fn batch_get_keeps_order_and_duplicates() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    store
        .set(&route_key("a"), &RouteValue::Counter(1), None)
        .await
        .unwrap();
    store
        .set(&route_key("b"), &RouteValue::Counter(2), None)
        .await
        .unwrap();
    let keys = [
        route_key("b"),
        route_key("a"),
        route_key("b"),
        route_key("missing"),
    ];
    let observed = store.batch_get(&keys).await.unwrap();
    assert_eq!(
        observed.value,
        vec![
            StoreRead::Value(RouteValue::Counter(2)),
            StoreRead::Value(RouteValue::Counter(1)),
            StoreRead::Value(RouteValue::Counter(2)),
            StoreRead::Missing,
        ]
    );
    assert!(observed.warning.is_none());
}

async fn increment_counter_from_missing_and_existing() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    let first = store
        .increment(&key, &RouteIncrement::Counter(5), None)
        .await
        .unwrap();
    assert_eq!(first.value, RouteValue::Counter(5));
    let second = store
        .increment(&key, &RouteIncrement::Counter(7), Some(120.0_f64))
        .await
        .unwrap();
    assert_eq!(second.value, RouteValue::Counter(12));
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Counter(12))
    );
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(120));
}

async fn increment_spend_is_exact() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("spend");
    store
        .increment(&key, &RouteIncrement::Spend(Decimal::new(1, 1)), None)
        .await
        .unwrap();
    let observed = store
        .increment(&key, &RouteIncrement::Spend(Decimal::new(2, 1)), None)
        .await
        .unwrap();
    assert_eq!(observed.value, RouteValue::Spend(Decimal::new(3, 1)));
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Spend(Decimal::new(3, 1)))
    );
}

async fn float_increments_and_pipeline() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("float");
    let single = store
        .increment_float(&key, 1.5_f64, None, false)
        .await
        .unwrap();
    assert!(same_f64(single.value, 1.5_f64));
    assert_eq!(server.raw("float").map(|entry| entry.1), Some(60));
    let entries = [
        FloatIncrement {
            key: route_key("first"),
            value: 1.5_f64,
            ttl_seconds: Some(30.0_f64),
        },
        FloatIncrement {
            key: route_key("second"),
            value: 2.25_f64,
            ttl_seconds: None,
        },
    ];
    let pipeline = store
        .increment_float_pipeline(&entries)
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(pipeline.len(), 2);
    assert!(same_f64(pipeline[0], 1.5_f64));
    assert!(same_f64(pipeline[1], 2.25_f64));
}

async fn empty_float_pipeline_is_none() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let observed = store.increment_float_pipeline(&[]).await.unwrap();
    assert_eq!(observed.value, None);
    assert!(observed.warning.is_none());
}

async fn compare_and_set_first_writer() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("affinity");
    let first = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let second = RouteValue::Pin(PinValue {
        model_id: "second".to_owned(),
    });
    let won = store
        .compare_and_set(&key, &StoreRead::Missing, &first, Some(120.0_f64))
        .await
        .unwrap();
    assert_eq!(
        won.value,
        CompareOutcome {
            swapped: true,
            current: StoreRead::Value(first.clone()),
        }
    );
    let lost = store
        .compare_and_set(&key, &StoreRead::Missing, &second, Some(120.0_f64))
        .await
        .unwrap();
    assert_eq!(
        lost.value,
        CompareOutcome {
            swapped: false,
            current: StoreRead::Value(first),
        }
    );
}

async fn compare_and_set_matching_value_swaps() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), Some(30.0_f64))
        .await
        .unwrap();
    let swapped = store
        .compare_and_set(
            &key,
            &StoreRead::Value(RouteValue::Counter(1)),
            &RouteValue::Counter(2),
            Some(10.0_f64),
        )
        .await
        .unwrap();
    assert_eq!(
        swapped.value,
        CompareOutcome {
            swapped: true,
            current: StoreRead::Value(RouteValue::Counter(2)),
        }
    );
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(10));
    compare_and_set_reports_the_winner(&store).await;
}

async fn compare_and_set_reports_the_winner(store: &super::super::RoutingStateStore) {
    let key = route_key("counter");
    let stale = store
        .compare_and_set(
            &key,
            &StoreRead::Value(RouteValue::Counter(1)),
            &RouteValue::Counter(3),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        stale.value,
        CompareOutcome {
            swapped: false,
            current: StoreRead::Value(RouteValue::Counter(2)),
        }
    );
}

async fn set_add_accumulates_members() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("members");
    let first = store
        .set_add(&key, &["a".to_owned(), "b".to_owned()], Some(60.0_f64))
        .await
        .unwrap();
    assert_eq!(first.value, vec!["a".to_owned(), "b".to_owned()]);
    let second = store
        .set_add(&key, &["b".to_owned(), "c".to_owned()], None)
        .await
        .unwrap();
    assert_eq!(second.value, vec!["b".to_owned(), "c".to_owned()]);
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Members(BTreeSet::from([
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
        ])))
    );
}

async fn delete_removes_value() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), None)
        .await
        .unwrap();
    store.delete(&key).await.unwrap();
    assert!(server.raw("counter").is_none());
    assert_eq!(store.get(&key).await.unwrap().value, StoreRead::Missing);
}

async fn ttl_read_and_refresh() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(1), Some(30.0_f64))
        .await
        .unwrap();
    assert_eq!(store.get_ttl(&key).await.unwrap().value, Some(30));
    assert!(store.refresh_ttl(&key, None).await.unwrap().value);
    assert_eq!(store.get_ttl(&key).await.unwrap().value, Some(60));
    let missing = route_key("missing");
    assert!(
        !store
            .refresh_ttl(&missing, Some(30.0_f64))
            .await
            .unwrap()
            .value
    );
    assert_eq!(store.get_ttl(&missing).await.unwrap().value, None);
}

async fn transport_failure_falls_back_for_reads() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.set_down(true);
    let single = store.get(&route_key("counter")).await.unwrap();
    assert_eq!(single.value, StoreRead::Missing);
    assert!(matches!(
        single.warning,
        Some(StoreWarning::Unavailable {
            operation: "async_get_cache",
            ..
        })
    ));
    let batch = store
        .batch_get(&[route_key("a"), route_key("b")])
        .await
        .unwrap();
    assert_eq!(batch.value, vec![StoreRead::Missing, StoreRead::Missing]);
    assert!(matches!(
        batch.warning,
        Some(StoreWarning::Unavailable {
            operation: "async_batch_get_cache",
            ..
        })
    ));
    let ttl = store.get_ttl(&route_key("counter")).await.unwrap();
    assert_eq!(ttl.value, None);
    assert!(matches!(
        ttl.warning,
        Some(StoreWarning::Unavailable {
            operation: "async_get_ttl",
            ..
        })
    ));
}

async fn transport_failure_fails_increments() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.set_down(true);
    let key = route_key("counter");
    assert!(matches!(
        store
            .increment(&key, &RouteIncrement::Counter(1), None)
            .await,
        Err(StateStoreError::Redis { .. } | StateStoreError::Reuse(_))
    ));
    assert!(matches!(
        store.increment_float(&key, 1.0_f64, None, false).await,
        Err(StateStoreError::Reuse(_) | StateStoreError::Counters { .. })
    ));
    assert!(matches!(
        store.refresh_ttl(&key, Some(30.0_f64)).await,
        Err(StateStoreError::Reuse(_))
    ));
}

async fn breaker_rejects_after_a_swallowed_failure() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(1, 30_000), &clock);
    server.set_down(true);
    let first = store.get(&route_key("counter")).await.unwrap();
    assert!(first.warning.is_some());
    assert!(matches!(
        store.get(&route_key("counter")).await,
        Err(StateStoreError::CircuitOpen { operation }) if operation == "async_get_cache"
    ));
    assert!(matches!(
        store
            .increment(&route_key("counter"), &RouteIncrement::Counter(1), None)
            .await,
        Err(StateStoreError::CircuitOpen { operation }) if operation == "async_increment"
    ));
}

async fn breaker_recovers_after_the_timeout() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(1, 30_000), &clock);
    server.set_down(true);
    store.get(&route_key("counter")).await.unwrap();
    clock.set_millis(30_001);
    server.set_down(false);
    let recovered = store.get(&route_key("counter")).await.unwrap();
    assert_eq!(recovered.value, StoreRead::Missing);
    assert!(matches!(
        recovered.warning,
        Some(StoreWarning::Breaker(BreakerEvent::Recovered))
    ));
}

async fn attach_redis_only_when_none_is_set() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = local_only_store(&clock);
    store
        .attach_redis(&redis_settings(&server, Some("first")))
        .unwrap();
    store
        .set(&route_key("key"), &RouteValue::Counter(1), None)
        .await
        .unwrap();
    assert!(server.raw("first:key").is_some());
    store
        .attach_redis(&redis_settings(&server, Some("second")))
        .unwrap();
    store
        .set(&route_key("other"), &RouteValue::Counter(2), None)
        .await
        .unwrap();
    assert!(server.raw("first:other").is_some());
    assert!(server.raw("second:other").is_none());
}

async fn local_backend_round_trips() {
    let clock = TestClock::new();
    let store = local_only_store(&clock);
    let key = route_key("counter");
    store
        .set(&key, &RouteValue::Counter(5), None)
        .await
        .unwrap();
    assert_eq!(
        store.get(&key).await.unwrap().value,
        StoreRead::Value(RouteValue::Counter(5))
    );
    let incremented = store
        .increment(&key, &RouteIncrement::Counter(2), None)
        .await
        .unwrap();
    assert_eq!(incremented.value, RouteValue::Counter(7));
    let swapped = store
        .compare_and_set(
            &key,
            &StoreRead::Value(RouteValue::Counter(7)),
            &RouteValue::Counter(8),
            None,
        )
        .await
        .unwrap();
    assert!(swapped.value.swapped);
    assert!(matches!(
        store.increment_float(&key, 1.0_f64, None, false).await,
        Err(StateStoreError::RedisRequired { .. })
    ));
    assert!(matches!(
        store.get_ttl(&key).await,
        Err(StateStoreError::RedisRequired { .. })
    ));
}

async fn credentials_stay_out_of_errors() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let password = "super-secret-redis-password";
    let mut redis = redis_settings(&server, None);
    redis.password = Some(secret(password));
    let settings = super::super::Settings {
        redis: Some(RedisSettings {
            password: Some(SecretString::from(password.to_owned())),
            ..redis
        }),
        breaker: breaker(5, 60_000),
    };
    let store = super::super::RoutingStateStore::new(
        super::support::local_store(),
        &settings,
        clock.breaker_clock(),
    )
    .unwrap();
    server.set_down(true);
    let error = store
        .increment(&route_key("counter"), &RouteIncrement::Counter(1), None)
        .await
        .unwrap_err();
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains(password));
    let debugged = format!("{settings:?}");
    assert!(!debugged.contains(password));
}
