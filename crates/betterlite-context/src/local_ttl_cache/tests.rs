#![cfg(test)]

use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use super::credential::{Credential, CredentialHandle, CredentialIdentity};
use super::dual::{CacheEvent, Diagnostics, DualLocalCache, EventKind, Ttl};
use super::{CacheError, CacheValue, LocalTtlCache, OrderedFloat, decode_cached_json};
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_redact::secret::Redactor;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn cache(clock: f64) -> LocalTtlCache {
    timed_cache(2, 1024, Arc::new(move || clock))
}

fn timed_cache(
    capacity: i64,
    maximum: i64,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
) -> LocalTtlCache {
    LocalTtlCache::with_clock(Some(capacity), Some(600.0_f64), Some(maximum), 1024, clock).unwrap()
}

#[rstest]
#[case::cache_defaults("cache_defaults")]
#[case::expiry_and_overwrite("expiry_and_overwrite")]
#[case::nonrefreshing_ttl("nonrefreshing_ttl")]
#[case::value_size_policy("value_size_policy")]
#[case::cached_json_projection("cached_json_projection")]
#[case::dual_local_defaults_errors("dual_local_defaults_errors")]
fn local_ttl_cache_cases(#[case] name: &str) {
    match name {
        "cache_defaults" => {
            let defaults =
                LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64)).unwrap();
            assert_eq!(defaults.limits.capacity, 200);
            assert_eq!(defaults.limits.default_ttl.to_bits(), 600.0_f64.to_bits());
            assert_eq!(defaults.limits.max_size_per_item, 1024);
            let zero =
                LocalTtlCache::with_clock(None, Some(0.0_f64), Some(0), 1024, Arc::new(|| 0.0_f64))
                    .unwrap();
            assert_eq!(zero.limits.default_ttl.to_bits(), 600.0_f64.to_bits());
            assert_eq!(zero.limits.max_size_per_item, 1024);
            let loaded =
                LocalTtlCache::with_clock(None, None, Some(0), 7, Arc::new(|| 0.0_f64)).unwrap();
            assert_eq!(loaded.limits.max_size_per_item, 7);
            assert!(
                defaults.values.is_empty()
                    && defaults.expiries.is_empty()
                    && defaults.heap.is_empty()
            );
            let mut disabled =
                LocalTtlCache::with_clock(Some(0), None, None, 1024, Arc::new(|| 0.0_f64)).unwrap();
            disabled
                .set("disabled", CacheValue::Integer(1), None)
                .unwrap();
            assert!(disabled.get("disabled").unwrap().is_none());
            let mut bounded =
                LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64)).unwrap();
            bounded
                .set(&"k".repeat(1024), CacheValue::Integer(1), None)
                .unwrap();
            assert!(bounded.get(&"k".repeat(1024)).unwrap().is_some());
            bounded
                .set(&"k".repeat(1025), CacheValue::Integer(1), None)
                .unwrap();
            assert!(bounded.get(&"k".repeat(1025)).unwrap().is_none());
            assert_eq!(
                OrderedFloat(1.0_f64).partial_cmp(&OrderedFloat(2.0_f64)),
                Some(Ordering::Less)
            );
        }
        "expiry_and_overwrite" => {
            let mut cache = cache(10.0_f64);
            cache
                .set("b", CacheValue::Integer(2), Some(5.0_f64))
                .unwrap();
            cache
                .set("a", CacheValue::Integer(1), Some(5.0_f64))
                .unwrap();
            cache
                .set("c", CacheValue::Integer(3), Some(5.0_f64))
                .unwrap();
            assert_eq!(cache.expiries.get("b").copied(), Some(15.0_f64));
            assert!(cache.get("a").unwrap().is_none());
            assert!(cache.get("b").unwrap().is_some());
            assert!(cache.get("c").unwrap().is_some());
            let clock = Arc::new(Mutex::new(0.0_f64));
            let expiry_source = Arc::clone(&clock);
            let mut expiring =
                timed_cache(3, 2048, Arc::new(move || *expiry_source.lock().unwrap()));
            expiring
                .set("expired", CacheValue::Integer(1), Some(1.0_f64))
                .unwrap();
            *clock.lock().unwrap() = 2.0_f64;
            expiring
                .set("replacement", CacheValue::Integer(2), Some(5.0_f64))
                .unwrap();
            assert!(expiring.expiries.get("expired").copied().is_none());
            assert_eq!(expiring.expiries.get("replacement").copied(), Some(7.0_f64));
        }
        "nonrefreshing_ttl" => {
            let mut cache = cache(0.0_f64);
            cache
                .set("key", CacheValue::Integer(1), Some(10.0_f64))
                .unwrap();
            cache
                .set("key", CacheValue::Integer(2), Some(100.0_f64))
                .unwrap();
            assert_eq!(cache.expiries.get("key").copied(), Some(10.0_f64));
            let equality_clock = Arc::new(Mutex::new(0.0_f64));
            let equality_source = Arc::clone(&equality_clock);
            let mut equality =
                timed_cache(2, 1024, Arc::new(move || *equality_source.lock().unwrap()));
            equality
                .set("key", CacheValue::Integer(1), Some(10.0_f64))
                .unwrap();
            *equality_clock.lock().unwrap() = 10.0_f64;
            equality.expiries.insert("key".to_owned(), 5.0_f64);
            equality
                .set("key", CacheValue::Integer(2), Some(100.0_f64))
                .unwrap();
            assert_eq!(equality.expiries.get("key").copied(), Some(110.0_f64));
            equality.expiries.insert("key".to_owned(), 10.0_f64);
            equality
                .set("key", CacheValue::Integer(3), Some(100.0_f64))
                .unwrap();
            assert_eq!(equality.expiries.get("key").copied(), Some(10.0_f64));
            cache.delete("key");
            cache
                .set("negative", CacheValue::Integer(1), Some(-1.0_f64))
                .unwrap();
            assert!(cache.get("negative").unwrap().is_none());
            assert!(cache.get("key").unwrap().is_none());
            let clock = Arc::new(Mutex::new(0.0_f64));
            let boundary_source = Arc::clone(&clock);
            let mut boundary =
                timed_cache(2, 1024, Arc::new(move || *boundary_source.lock().unwrap()));
            boundary
                .set("boundary", CacheValue::Integer(1), Some(1.0_f64))
                .unwrap();
            *clock.lock().unwrap() = 1.0_f64;
            assert!(boundary.get("boundary").unwrap().is_some());
            *clock.lock().unwrap() = 2.0_f64;
            assert!(boundary.get("boundary").unwrap().is_none());
        }
        "value_size_policy" => {
            let mut negative =
                LocalTtlCache::with_clock(None, None, Some(-1), 1024, Arc::new(|| 0.0_f64))
                    .unwrap();
            negative
                .set("empty", CacheValue::Bytes(vec![]), None)
                .unwrap();
            assert_eq!(negative.get("empty").unwrap(), None);
            let mut loaded_negative =
                LocalTtlCache::with_clock(None, None, None, -1, Arc::new(|| 0.0_f64)).unwrap();
            loaded_negative
                .set("empty", CacheValue::Bytes(vec![]), None)
                .unwrap();
            assert_eq!(loaded_negative.get("empty").unwrap(), None);
            let mut large_limit = timed_cache(1, i64::MAX, Arc::new(|| 0.0_f64));
            large_limit
                .set("empty", CacheValue::Bytes(vec![]), None)
                .unwrap();
            assert_eq!(
                large_limit.get("empty").unwrap(),
                Some(CacheValue::Bytes(vec![]))
            );
            let mut cache = timed_cache(1, 1, Arc::new(|| 0.0_f64));
            cache
                .set("small", CacheValue::Text("ok".to_owned()), None)
                .unwrap();
            assert!(cache.get("small").unwrap().is_some());
            cache
                .set("large", CacheValue::Text("x".repeat(2000)), None)
                .unwrap();
            assert!(cache.get("large").unwrap().is_none());
            assert!(cache.get("small").unwrap().is_none());
            let mut ceiling = timed_cache(2, 2048, Arc::new(|| 0.0_f64));
            ceiling
                .set("ceiling", CacheValue::Text("x".repeat(2050)), None)
                .unwrap();
            assert!(ceiling.get("ceiling").unwrap().is_some());
        }
        "cached_json_projection" => {
            let mut stored = cache(0.0_f64);
            stored
                .set("json", CacheValue::Text("false".to_owned()), None)
                .unwrap();
            assert_eq!(
                stored.get("json").unwrap(),
                Some(CacheValue::Boolean(false))
            );
            assert_eq!(decode_cached_json(b"false"), CacheValue::Boolean(false));
            assert_eq!(
                decode_cached_json(b"not-json"),
                CacheValue::Text("not-json".to_owned())
            );
        }
        "dual_local_defaults_errors" => dual_local_defaults_errors(),
        _ => assert!(name.is_empty()),
    }
}

fn diagnostics(events: &Arc<Mutex<Vec<CacheEvent>>>) -> Diagnostics {
    let sink = Arc::clone(events);
    Diagnostics {
        redactor: Arc::new(Redactor::new(16).unwrap()),
        emit: Arc::new(move |event| sink.lock().unwrap().push(event)),
    }
}

fn dual_local_defaults_errors() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let memory = Arc::new(Mutex::new(cache(0.0_f64)));
    let settings = Settings {
        global_max_item_kib: 7,
        global_ttl: Some(40.0_f64),
    };
    let dual = DualLocalCache::new(
        Some(Arc::clone(&memory)),
        Some(0.0_f64),
        &settings,
        Arc::new(|| 0.0_f64),
        diagnostics(&events),
    );
    assert!(Arc::ptr_eq(&memory, &dual.memory));
    dual.set("absent", CacheValue::Boolean(false), Ttl::Absent);
    dual.set("none", CacheValue::Integer(0), Ttl::Explicit(None));
    assert_eq!(
        memory.lock().unwrap().expiries.get("absent"),
        Some(&40.0_f64)
    );
    assert_eq!(
        memory.lock().unwrap().expiries.get("none"),
        Some(&600.0_f64)
    );
    assert_eq!(dual.get("absent"), Some(CacheValue::Boolean(false)));
    assert_eq!(dual.get("none"), Some(CacheValue::Integer(0)));
    dual.set("zero", CacheValue::Null, Ttl::Explicit(Some(0.0_f64)));
    assert_eq!(memory.lock().unwrap().expiries.get("zero"), Some(&0.0_f64));
    assert_eq!(dual.get("zero"), None);
    assert_eq!(dual.get("missing"), None);
    dual_default_construction(&events, &settings);
    dual_local_failures(&memory, &dual, &events);
}

fn dual_default_construction(events: &Arc<Mutex<Vec<CacheEvent>>>, settings: &Settings) {
    let default = DualLocalCache::new(
        None,
        Some(3.0_f64),
        settings,
        Arc::new(|| 0.0_f64),
        diagnostics(events),
    );
    assert_eq!(default.memory.lock().unwrap().limits.capacity, 200);
    default.set("override", CacheValue::Integer(1), Ttl::Absent);
    assert_eq!(
        default.memory.lock().unwrap().expiries.get("override"),
        Some(&3.0_f64)
    );
    let unset = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics(events),
    );
    assert_eq!(unset.default_in_memory_ttl, None);
    unset.set("default", CacheValue::Integer(1), Ttl::Absent);
    assert_eq!(
        unset.memory.lock().unwrap().expiries.get("default"),
        Some(&600.0_f64)
    );
}

fn dual_local_failures(
    memory: &Arc<Mutex<LocalTtlCache>>,
    dual: &DualLocalCache,
    events: &Arc<Mutex<Vec<CacheEvent>>>,
) {
    memory.lock().unwrap().heap.clear();
    dual.set("failure", CacheValue::Integer(2), Ttl::Absent);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.message.starts_with("set cache:"))
    );
    catch_unwind(AssertUnwindSafe(|| {
        let _guard = memory.lock().unwrap();
        panic!("injected cache owner failure");
    }))
    .unwrap_err();
    assert_eq!(dual.get("none"), None);
    dual.set("poisoned", CacheValue::Integer(1), Ttl::Absent);
    let captured = events.lock().unwrap();
    assert!(captured.iter().any(|event| {
        event.kind == EventKind::Error
            && event
                .message
                .starts_with("LiteLLM Cache: exception in get_cache")
    }));
    assert!(
        captured
            .iter()
            .all(|event| event.message.ends_with("REDACTED")),
        "{captured:?}"
    );
    drop(captured);
}

fn credential(secret: &str) -> CacheValue {
    CacheValue::Credential(CredentialHandle(Arc::new(Credential {
        token: SecretString::from(secret),
        headers: BTreeMap::from([("authorization".to_owned(), SecretString::from(secret))]),
    })))
}

#[test]
fn local_ttl_cache_credential_identity_isolation() {
    let first = CredentialIdentity {
        authority: "ab".to_owned(),
        principal: "c".to_owned(),
    };
    let second = CredentialIdentity {
        authority: "a".to_owned(),
        principal: "bc".to_owned(),
    };
    assert_eq!(
        format!("{}{}", first.authority, first.principal),
        format!("{}{}", second.authority, second.principal)
    );
    let mut cache = cache(0.0_f64);
    let first_key = first.cache_key("token");
    let second_key = second.cache_key("token");
    cache
        .set(&first_key, credential("fixture-first"), None)
        .unwrap();
    cache
        .set(&second_key, credential("fixture-second"), None)
        .unwrap();
    let first_hit = cache.get(&first_key).unwrap();
    let second_hit = cache.get(&second_key).unwrap();
    assert_ne!(first_hit, second_hit);
    assert_eq!(first_hit, first_hit.clone());
    assert_credential(first_hit, "fixture-first");
    assert_credential(second_hit, "fixture-second");
}

fn assert_credential(value: Option<CacheValue>, expected: &str) {
    let Some(CacheValue::Credential(handle)) = value else {
        panic!("missing credential")
    };
    assert_eq!(handle.0.token.expose_secret(), expected);
    assert_eq!(
        handle
            .0
            .headers
            .get("authorization")
            .map(ExposeSecret::expose_secret),
        Some(expected)
    );
}

#[test]
fn local_ttl_cache_secret_safe_failure() {
    let mut cache = cache(0.0_f64);
    cache.limits.capacity = 1;
    cache
        .set("key", credential("fixture-secret"), None)
        .unwrap();
    assert!(!format!("{cache:?} {:?}", cache.get("key").unwrap()).contains("fixture-secret"));
    assert!(format!("{cache:?}").contains("LocalTtlCache"));
    cache.heap.clear();
    let error = cache
        .set("replacement", credential("fixture-secret"), None)
        .unwrap_err();
    assert_eq!(error, CacheError::MissingExpiry);
    assert!(!format!("{error:?} {error} {cache:?}").contains("fixture-secret"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let dual = DualLocalCache::new(
        Some(Arc::new(Mutex::new(cache))),
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics(&events),
    );
    dual.set("fixture-secret", credential("fixture-secret"), Ttl::Absent);
    assert!(dual.get("key").is_some());
    let captured = events.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|event| event.message.starts_with("set cache:"))
    );
    assert!(!format!("{captured:?}").contains("fixture-secret"));
    drop(captured);
    LocalTtlCache::with_clock(Some(-1), None, None, 1024, Arc::new(|| 0.0_f64)).unwrap_err();
}

#[test]
fn replays_the_cached_json_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cached_json");
    let entries = fs::read_dir(corpus)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.len() >= 4);
    assert!(entries.into_iter().all(|entry| {
        let bytes = fs::read(entry.path()).unwrap();
        let expected = match entry.file_name().to_str().unwrap() {
            "valid" => CacheValue::Boolean(false),
            "boundary" => CacheValue::Null,
            "invalid-utf8" => CacheValue::Bytes(bytes.clone()),
            "malformed" | "truncated" => {
                CacheValue::Text(String::from_utf8(bytes.clone()).unwrap())
            }
            other => panic!("unclassified seed: {other}"),
        };
        decode_cached_json(&bytes) == expected
    }));
}

#[rstest]
#[case::null("null", CacheValue::Null)]
#[case::zero("0", CacheValue::Integer(0))]
#[case::array("[false]", CacheValue::Array(vec![CacheValue::Boolean(false)]))]
#[case::fallback("bad", CacheValue::Text("bad".to_owned()))]
fn projects_stored_json(#[case] text: &str, #[case] expected: CacheValue) {
    let mut cache = cache(0.0_f64);
    cache
        .set("json", CacheValue::Text(text.to_owned()), None)
        .unwrap();
    assert_eq!(cache.get("json").unwrap(), Some(expected.clone()));
    cache
        .set("bytes", CacheValue::Bytes(text.as_bytes().to_vec()), None)
        .unwrap();
    let byte_expected = if text == "bad" {
        CacheValue::Bytes(text.as_bytes().to_vec())
    } else {
        expected
    };
    assert_eq!(cache.get("bytes").unwrap(), Some(byte_expected));
}

#[test]
fn rejects_invalid_clock_and_size_boundaries() {
    let mut invalid = cache(f64::NAN);
    assert_eq!(invalid.get("key"), Err(CacheError::InvalidClock));
    assert_eq!(
        invalid.set("key", CacheValue::Null, None),
        Err(CacheError::InvalidClock)
    );
    let mut bounded = timed_cache(2, 1, Arc::new(|| 0.0_f64));
    bounded
        .set("exact", CacheValue::Text("x".repeat(1022)), None)
        .unwrap();
    assert!(bounded.get("exact").unwrap().is_some());
    bounded
        .set("over", CacheValue::Text("x".repeat(1023)), None)
        .unwrap();
    assert!(bounded.get("over").unwrap().is_none());
    bounded
        .set("handle", credential(&"s".repeat(1024)), None)
        .unwrap();
    assert!(bounded.get("handle").unwrap().is_none());
    bounded
        .set("fit", credential(&"s".repeat(505)), None)
        .unwrap();
    assert!(bounded.get("fit").unwrap().is_some());
    bounded
        .set("over", credential(&"s".repeat(506)), None)
        .unwrap();
    assert!(bounded.get("over").unwrap().is_none());
}

#[test]
fn bounds_cached_json_input() {
    let mut bytes = b"false".to_vec();
    bytes.resize(super::MAX_VALUE_BYTES, b' ');
    assert_eq!(decode_cached_json(&bytes), CacheValue::Boolean(false));
    bytes.push(b' ');
    assert_eq!(decode_cached_json(&bytes), CacheValue::Null);
}
