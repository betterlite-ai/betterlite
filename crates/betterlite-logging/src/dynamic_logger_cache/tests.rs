#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use secrecy::SecretString;

use super::{DynamicLoggerCache, RemovalEffect, get_cache_key};

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

fn fake_clock(time: &Arc<Mutex<f64>>) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    let shared_time = Arc::clone(time);
    Arc::new(move || {
        *shared_time
            .lock()
            .expect("fixture clock lock is not poisoned")
    })
}

fn capturing_removal(log: &Arc<Mutex<Vec<FixtureLogger>>>) -> RemovalEffect<FixtureLogger> {
    let shared_log = Arc::clone(log);
    Arc::new(move |value: FixtureLogger| {
        shared_log
            .lock()
            .expect("fixture capture lock is not poisoned")
            .push(value);
    })
}

fn cache_with(
    time: &Arc<Mutex<f64>>,
    stopped: &Arc<Mutex<Vec<FixtureLogger>>>,
) -> DynamicLoggerCache<FixtureLogger> {
    DynamicLoggerCache::new(capturing_removal(stopped), fake_clock(time))
}

fn set_first_two_hundred_credentials(
    cache: &DynamicLoggerCache<FixtureLogger>,
    time: &Arc<Mutex<f64>>,
) {
    (0_u32..200_u32).for_each(|index| {
        let credential = format!("team-{index}");
        let creds = credentials(&[("api_key", &credential)]);
        *time.lock().expect("fixture clock lock is not poisoned") = f64::from(index);
        let name = if index == 0 { "earliest" } else { "filler" };
        cache
            .set_cache(&creds, "langfuse", FixtureLogger { name })
            .expect("set succeeds");
    });
}

#[test]
fn set_cache_stops_the_live_value_it_replaces() {
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

    let value = cache.get_cache(&creds, "langfuse").expect("get succeeds");
    assert_eq!(value, Some(FixtureLogger { name: "second" }));
    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [FixtureLogger { name: "first" }]
    );
}

#[test]
fn set_cache_keeps_the_original_expiry_when_overwriting_a_live_entry() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "first" })
        .expect("set succeeds");
    *time.lock().expect("fixture clock lock is not poisoned") = 60.0_f64;
    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "second" })
        .expect("set succeeds");

    *time.lock().expect("fixture clock lock is not poisoned") = 3600.0_f64;
    let still_live = cache.get_cache(&creds, "langfuse").expect("get succeeds");
    assert_eq!(still_live, Some(FixtureLogger { name: "second" }));

    *time.lock().expect("fixture clock lock is not poisoned") = 3600.5_f64;
    let expired = cache.get_cache(&creds, "langfuse").expect("get succeeds");
    assert_eq!(expired, None);
    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [
            FixtureLogger { name: "first" },
            FixtureLogger { name: "second" }
        ]
    );
}

#[test]
fn new_cache_keeps_an_entry_for_exactly_one_hour() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "client" })
        .expect("set succeeds");

    *time.lock().expect("fixture clock lock is not poisoned") = 3600.0_f64;
    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "client" })
    );

    *time.lock().expect("fixture clock lock is not poisoned") = 3600.5_f64;
    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        None
    );
}

#[test]
fn get_cache_evicts_and_stops_the_value_once_the_ttl_expires() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "datadog", FixtureLogger { name: "metrics" })
        .expect("set succeeds");
    *time.lock().expect("fixture clock lock is not poisoned") = 3601.0_f64;

    let value = cache.get_cache(&creds, "datadog").expect("get succeeds");
    assert_eq!(value, None);
    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [FixtureLogger { name: "metrics" }]
    );
}

#[test]
fn explicit_eviction_stops_the_cached_value() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "new_relic", FixtureLogger { name: "metrics" })
        .expect("set succeeds");
    cache.evict(&creds, "new_relic").expect("evict succeeds");

    let value = cache.get_cache(&creds, "new_relic").expect("get succeeds");
    assert_eq!(value, None);
    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [FixtureLogger { name: "metrics" }]
    );
}

#[test]
fn explicit_eviction_of_a_missing_key_is_a_harmless_no_op() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache.evict(&creds, "new_relic").expect("evict succeeds");
    assert!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn explicit_eviction_survives_a_panicking_stop_without_propagating() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let cache = DynamicLoggerCache::new(
        Arc::new(|_value: FixtureLogger| panic!("stop() is broken")),
        fake_clock(&time),
    );
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "new_relic", FixtureLogger { name: "metrics" })
        .expect("set succeeds");

    cache
        .evict(&creds, "new_relic")
        .expect("a panicking stop() must not fail eviction");
}

#[test]
fn get_cache_key_is_deterministic_for_the_same_input() {
    let creds = credentials(&[("api_key", "k")]);
    let first = get_cache_key(&creds, "langfuse").expect("key hashes");
    let second = get_cache_key(&creds, "langfuse").expect("key hashes");
    assert_eq!(first, second);
}

enum Relation {
    Equal,
    Different,
}

#[rstest]
#[case::overridden_service_name_key_is_replaced_by_the_parameter(
    credentials(&[("service_name", "wrong"), ("api_key", "k")]),
    "real",
    credentials(&[("api_key", "k")]),
    "real",
    Relation::Equal,
)]
#[case::different_service_names_diverge(
    credentials(&[("api_key", "k")]),
    "langfuse",
    credentials(&[("api_key", "k")]),
    "datadog",
    Relation::Different,
)]
#[case::different_credentials_diverge(
    credentials(&[("api_key", "k1")]),
    "langfuse",
    credentials(&[("api_key", "k2")]),
    "langfuse",
    Relation::Different,
)]
#[case::empty_credentials_still_hash(
    credentials(&[]),
    "langfuse",
    credentials(&[]),
    "datadog",
    Relation::Different,
)]
fn get_cache_key_cases(
    #[case] left_credentials: BTreeMap<String, SecretString>,
    #[case] left_service: &str,
    #[case] right_credentials: BTreeMap<String, SecretString>,
    #[case] right_service: &str,
    #[case] relation: Relation,
) {
    let left = get_cache_key(&left_credentials, left_service).expect("key hashes");
    let right = get_cache_key(&right_credentials, right_service).expect("key hashes");
    match relation {
        Relation::Equal => assert_eq!(left, right),
        Relation::Different => assert_ne!(left, right),
    }
}

#[test]
fn cache_holds_an_arbitrary_value_type_with_no_reference_to_any_logger_crate() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let creds = credentials(&[("api_key", "k")]);

    cache
        .set_cache(&creds, "langfuse", FixtureLogger { name: "direct" })
        .expect("set succeeds");
    assert_eq!(
        cache.get_cache(&creds, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "direct" })
    );
}

#[test]
fn set_cache_evicts_and_stops_an_expired_entry_without_ever_reading_it() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let first = credentials(&[("api_key", "a")]);
    let second = credentials(&[("api_key", "b")]);

    cache
        .set_cache(&first, "datadog", FixtureLogger { name: "a" })
        .expect("set succeeds");
    *time.lock().expect("fixture clock lock is not poisoned") = 3601.0_f64;

    cache
        .set_cache(&second, "datadog", FixtureLogger { name: "b" })
        .expect("set succeeds");

    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [FixtureLogger { name: "a" }]
    );
}

#[test]
fn new_cache_evicts_the_earliest_entry_on_the_201st_credential_set() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let cache = cache_with(&time, &stopped);
    let earliest = credentials(&[("api_key", "team-0")]);

    set_first_two_hundred_credentials(&cache, &time);
    assert!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .is_empty()
    );

    let newest = credentials(&[("api_key", "team-200")]);
    *time.lock().expect("fixture clock lock is not poisoned") = 200.0_f64;
    cache
        .set_cache(&newest, "langfuse", FixtureLogger { name: "newest" })
        .expect("set succeeds");
    assert_eq!(
        stopped
            .lock()
            .expect("fixture capture lock is not poisoned")
            .as_slice(),
        [FixtureLogger { name: "earliest" }]
    );
    assert_eq!(
        cache
            .get_cache(&earliest, "langfuse")
            .expect("get succeeds"),
        None
    );
    let next = credentials(&[("api_key", "team-1")]);
    assert_eq!(
        cache.get_cache(&next, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "filler" })
    );
    assert_eq!(
        cache.get_cache(&newest, "langfuse").expect("get succeeds"),
        Some(FixtureLogger { name: "newest" })
    );
}
