use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_context::dynamic_logging_cache::{Clock, DynamicLoggingCache, RemovalEffect};
use secrecy::SecretString;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FixtureLogger {
    name: &'static str,
}

fn credentials(pairs: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), SecretString::from(*value)))
        .collect()
}

type TestTime = Arc<Mutex<f64>>;

fn fake_clock(time: &TestTime) -> Clock {
    let shared = Arc::clone(time);
    Arc::new(move || *shared.lock().expect("fixture clock lock is not poisoned"))
}

fn advance(time: &TestTime, seconds: f64) {
    let mut now = time.lock().expect("fixture clock lock is not poisoned");
    *now = seconds.mul_add(1.0_f64, *now);
}

fn capturing_removal(stopped: &Arc<Mutex<Vec<FixtureLogger>>>) -> RemovalEffect<FixtureLogger> {
    let shared = Arc::clone(stopped);
    Arc::new(move |logger: FixtureLogger| {
        shared
            .lock()
            .expect("fixture capture lock is not poisoned")
            .push(logger);
    })
}

fn cache_with(
    time: &TestTime,
    stopped: &Arc<Mutex<Vec<FixtureLogger>>>,
) -> DynamicLoggingCache<FixtureLogger> {
    DynamicLoggingCache::new(capturing_removal(stopped), fake_clock(time))
        .expect("the fixture cache is built")
}

fn stopped_loggers(stopped: &Arc<Mutex<Vec<FixtureLogger>>>) -> Vec<FixtureLogger> {
    stopped
        .lock()
        .expect("fixture capture lock is not poisoned")
        .clone()
}

fn set_first_two_hundred_credentials(cache: &DynamicLoggingCache<FixtureLogger>, time: &TestTime) {
    (0..200_u32).for_each(|index| {
        let credential = format!("team-{index}");
        let creds = credentials(&[("api_key", &credential)]);
        let mut now = time.lock().expect("fixture clock lock is not poisoned");
        *now = f64::from(index);
        drop(now);
        let name = if index == 0 { "earliest" } else { "filler" };
        cache
            .set_cache(&creds, "langfuse", FixtureLogger { name })
            .expect("set succeeds");
    });
}

#[test]
fn dynamic_logging_cache_reuses_the_client_for_the_same_credentials_and_service() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "first" })
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "first" })
    );
    assert!(stopped_loggers(&stopped).is_empty());
}

#[test]
fn dynamic_logging_cache_separates_clients_by_credentials_and_service_name() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);

    cache
        .set_cache(
            &credentials(&[("api_key", "k1")]),
            "langfuse",
            FixtureLogger { name: "own" },
        )
        .expect("set succeeds");

    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "k2")]), "langfuse")
            .expect("get succeeds"),
        None
    );
    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "k1")]), "datadog")
            .expect("get succeeds"),
        None
    );
}

#[test]
fn dynamic_logging_cache_keeps_a_logging_client_for_exactly_one_hour() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "client" })
        .expect("set succeeds");

    advance(&time, 3600.0_f64);
    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "client" })
    );

    advance(&time, 0.5_f64);
    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        None
    );
    assert_eq!(
        stopped_loggers(&stopped),
        [FixtureLogger { name: "client" }]
    );
}

#[test]
fn dynamic_logging_cache_stops_the_client_it_replaces() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "first" })
        .expect("set succeeds");
    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "second" })
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "second" })
    );
    assert_eq!(stopped_loggers(&stopped), [FixtureLogger { name: "first" }]);
}

#[test]
fn dynamic_logging_cache_evicts_the_earliest_entry_on_the_two_hundred_first_set() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    set_first_two_hundred_credentials(&cache, &time);
    assert!(stopped_loggers(&stopped).is_empty());

    cache
        .set_cache(
            &credentials(&[("api_key", "team-200")]),
            "langfuse",
            FixtureLogger { name: "newest" },
        )
        .expect("set succeeds");

    assert_eq!(
        stopped_loggers(&stopped),
        [FixtureLogger { name: "earliest" }]
    );
    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "team-0")]), "langfuse")
            .expect("get succeeds"),
        None
    );
    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "team-1")]), "langfuse")
            .expect("get succeeds"),
        Some(FixtureLogger { name: "filler" })
    );
    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "team-200")]), "langfuse")
            .expect("get succeeds"),
        Some(FixtureLogger { name: "newest" })
    );
}

#[test]
fn dynamic_logging_cache_survives_a_panicking_stop_during_eviction() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = DynamicLoggingCache::new(
        Arc::new(|_logger: FixtureLogger| panic!("stop() is broken")),
        fake_clock(&time),
    )
    .expect("the cache is built");
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "client" })
        .expect("set succeeds");
    advance(&time, 3601.0_f64);

    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        None
    );
}

#[test]
fn logging_client_without_a_stop_effect_is_still_evicted() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = DynamicLoggingCache::new(Arc::new(|_logger: FixtureLogger| {}), fake_clock(&time))
        .expect("the cache is built");
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "datadog", FixtureLogger { name: "client" })
        .expect("set succeeds");
    advance(&time, 3601.0_f64);

    assert_eq!(
        cache.get_cache(&creds, "datadog").expect("get succeeds"),
        None
    );
}

#[test]
fn dynamic_logging_cache_separates_empty_credentials_by_service_name() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let empty = BTreeMap::new();

    cache
        .set_cache(&empty, "langfuse", FixtureLogger { name: "langfuse" })
        .expect("set succeeds");

    assert_eq!(
        cache.get_cache(&empty, "datadog").expect("get succeeds"),
        None
    );
    assert_eq!(
        cache.get_cache(&empty, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "langfuse" })
    );
}

#[test]
fn dynamic_logging_cache_replaces_a_service_name_credential_with_the_parameter() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);

    cache
        .set_cache(
            &credentials(&[("service_name", "wrong"), ("api_key", "k")]),
            "real",
            FixtureLogger { name: "stored" },
        )
        .expect("set succeeds");

    assert_eq!(
        cache
            .get_cache(&credentials(&[("api_key", "k")]), "real")
            .expect("get succeeds"),
        Some(FixtureLogger { name: "stored" })
    );
}
