use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use secrecy::SecretString;

use betterlite_config::sections::routing_state_store::Settings as ConfigSettings;
use betterlite_config::sections::routing_state_store::connection::Connection;

use super::super::error::{StateStoreError, StoreWarning};
use super::super::{RedisSettings, RoutingStateStore, Settings};
use super::support::{TestClock, breaker, route_key, same_f64, secret, seed, settings, store};
use crate::redis_failure_policy::BreakerEvent;
use crate::routing_state_store::test_support::FakeRedis;
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
        "batch_get_empty_is_empty" => batch_get_empty_is_empty().await,
        "increment_counter_from_missing_and_existing" => {
            increment_counter_from_missing_and_existing().await;
        }
        "increment_spend_is_exact" => increment_spend_is_exact().await,
        "float_increments_and_pipeline" => float_increments_and_pipeline().await,
        "empty_float_pipeline_is_none" => empty_float_pipeline_is_none().await,
        "compare_and_set_first_writer" => compare_and_set_first_writer().await,
        "compare_and_set_matching_value_swaps" => compare_and_set_matching_value_swaps().await,
        "ttl_read_and_refresh" => ttl_read_and_refresh().await,
        _ => return false,
    }
    true
}

async fn failure_cases(name: &str) -> bool {
    if name == "transport_failure_falls_back_for_reads" {
        transport_failure_falls_back_for_reads().await;
        return true;
    }
    if name == "transport_failure_fails_increments" {
        transport_failure_fails_increments().await;
        return true;
    }
    if name == "breaker_rejects_after_a_swallowed_failure" {
        breaker_rejects_after_a_swallowed_failure().await;
        return true;
    }
    if name == "breaker_recovers_after_the_timeout" {
        breaker_recovers_after_the_timeout().await;
        return true;
    }
    if name == "config_mapping_defaults" {
        config_mapping_defaults();
        return true;
    }
    if name == "credentials_stay_out_of_errors" {
        credentials_stay_out_of_errors().await;
        return true;
    }
    if name == "url_credentials_stay_out_of_errors" {
        url_credentials_stay_out_of_errors().await;
        return true;
    }
    if name == "redis_environment_reaches_the_connection" {
        redis_environment_reaches_the_connection().await;
        return true;
    }
    false
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
        let key = format!("value-{index}");
        seed(&server, &key, value, -1);
        let observed = store.get(&route_key(&key)).await.unwrap();
        assert_eq!(observed.value, StoreRead::Value(value.clone()));
        assert!(observed.warning.is_none());
    }
}

async fn namespace_prefixes_every_key() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, Some("router"), breaker(5, 60_000), &clock);
    seed(&server, "router:counter", &RouteValue::Counter(1), -1);
    let observed = store.get(&route_key("counter")).await.unwrap();
    assert_eq!(observed.value, StoreRead::Value(RouteValue::Counter(1)));
    let already_prefixed = store.get(&route_key("router:counter")).await.unwrap();
    assert_eq!(
        already_prefixed.value,
        StoreRead::Value(RouteValue::Counter(1))
    );
    assert!(server.raw("counter").is_none());
}

async fn batch_get_keeps_order_and_duplicates() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "a", &RouteValue::Counter(1), -1);
    seed(&server, "b", &RouteValue::Counter(2), -1);
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

async fn batch_get_empty_is_empty() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let observed = store.batch_get(&[]).await.unwrap();
    assert!(observed.value.is_empty());
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
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(60));
    let second = store
        .increment(&key, &RouteIncrement::Counter(7), Some(120.0_f64))
        .await
        .unwrap();
    assert_eq!(second.value, RouteValue::Counter(12));
    assert_eq!(server.raw("counter").map(|entry| entry.1), Some(60));
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
    let read = store.get(&key).await.unwrap();
    assert_eq!(
        read.value,
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
        (route_key("first"), 1.5_f64, Some(30.0_f64)),
        (route_key("second"), 2.25_f64, None),
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
    seed(&server, "counter", &RouteValue::Counter(1), 30);
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

async fn ttl_read_and_refresh() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    seed(&server, "counter", &RouteValue::Counter(1), 30);
    let key = route_key("counter");
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

fn config_section(
    url: Option<&str>,
    host: Option<&str>,
    port: Option<i64>,
    environment: BTreeMap<String, SecretString>,
) -> ConfigSettings {
    ConfigSettings {
        redis_url: url.map(secret),
        redis_host: host.map(str::to_owned),
        redis_port: port,
        redis_password: Some(secret("pw")),
        namespace: None,
        connection: Connection::default(),
        socket_timeout: 5.0_f64,
        pool_timeout_seconds: 5,
        redis_environment: environment,
    }
}

fn config_mapping_defaults() {
    let mut named = config_section(None, Some("cache"), Some(6380), BTreeMap::new());
    named.namespace = Some("router".to_owned());
    named.socket_timeout = 1.5_f64;
    named.pool_timeout_seconds = 9;
    let mapped = RedisSettings::try_from(&named).unwrap();
    assert_eq!(mapped.host.as_deref(), Some("cache"));
    assert_eq!(mapped.port, Some(6380));
    assert_eq!(mapped.namespace.as_deref(), Some("router"));
    assert!(same_f64(mapped.socket_timeout, 1.5_f64));
    assert_eq!(mapped.pool_timeout_seconds, 9);
    let with_url = config_section(Some("redis://cache:6379"), None, None, BTreeMap::new());
    RedisSettings::try_from(&with_url).unwrap();
    let overlay = BTreeMap::from([
        ("REDIS_URL".to_owned(), secret("redis://cache:6379")),
        ("REDIS_HOST".to_owned(), secret("cache")),
        ("REDIS_PORT".to_owned(), secret("6379")),
    ]);
    let not_configured = [
        config_section(None, None, None, BTreeMap::new()),
        config_section(None, Some("cache"), None, BTreeMap::new()),
        config_section(None, None, Some(6380), BTreeMap::new()),
        config_section(None, None, None, overlay),
    ];
    for section in &not_configured {
        assert!(matches!(
            RedisSettings::try_from(section),
            Err(StateStoreError::NotConfigured)
        ));
    }
}

async fn redis_environment_reaches_the_connection() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let mut redis = settings(&server, None, breaker(5, 60_000)).redis;
    redis.environment = BTreeMap::from([
        ("REDIS_USERNAME".to_owned(), secret("env-user")),
        ("REDIS_PASSWORD".to_owned(), secret("env-password")),
        ("REDIS_DB".to_owned(), secret("3")),
    ]);
    let settings = Settings {
        redis,
        breaker: breaker(5, 60_000),
    };
    let store = RoutingStateStore::new(&settings, clock.breaker_clock()).unwrap();
    store.get(&route_key("counter")).await.unwrap();
    let handshakes = server.handshakes();
    assert!(
        handshakes.contains(&vec![
            "AUTH".to_owned(),
            "env-user".to_owned(),
            "env-password".to_owned()
        ]),
        "{handshakes:?}"
    );
    assert!(
        handshakes.contains(&vec!["SELECT".to_owned(), "3".to_owned()]),
        "{handshakes:?}"
    );
}

async fn credentials_stay_out_of_errors() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let password = "super-secret-redis-password";
    let mut redis = settings(&server, None, breaker(5, 60_000)).redis;
    redis.password = Some(secret(password));
    let settings = Settings {
        redis,
        breaker: breaker(5, 60_000),
    };
    let store = RoutingStateStore::new(&settings, clock.breaker_clock()).unwrap();
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

async fn url_credentials_stay_out_of_errors() {
    let password = "super-secret-redis-password";
    let rejecting = FakeRedis::start().await;
    rejecting.reject_auth(true);
    let unreachable = FakeRedis::start().await;
    let unreachable_port = unreachable.port();
    drop(unreachable);
    for port in [rejecting.port(), unreachable_port] {
        let clock = TestClock::new();
        let mut redis = settings(&rejecting, None, breaker(5, 60_000)).redis;
        redis.host = None;
        redis.port = None;
        redis.url = Some(secret(&format!("redis://:{password}@127.0.0.1:{port}")));
        let settings = Settings {
            redis,
            breaker: breaker(5, 60_000),
        };
        let store = RoutingStateStore::new(&settings, clock.breaker_clock()).unwrap();
        let error = store
            .increment(&route_key("counter"), &RouteIncrement::Counter(1), None)
            .await
            .unwrap_err();
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains(password), "{rendered}");
        assert!(!format!("{settings:?}").contains(password));
    }
    assert!(rejecting.handshakes().iter().any(|command| {
        command
            .first()
            .is_some_and(|name| name.eq_ignore_ascii_case("AUTH"))
    }));
}
