#![cfg(test)]

use std::collections::BTreeMap;
use std::env::{current_dir, set_current_dir};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::scope;

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;
use rusqlite::Connection;
use tempfile::TempDir;

use super::{Clock, DEFAULT_DISK_CACHE_DIRECTORY, DiskCache, DiskError};
use crate::coordinator::AsyncCacheProtocol as _;

fn clock_at(now: Arc<Mutex<f64>>) -> Clock {
    Arc::new(move || *now.lock().unwrap_or_else(PoisonError::into_inner))
}

fn temp_cache(start: f64) -> (TempDir, DiskCache, Arc<Mutex<f64>>) {
    let directory = TempDir::new().unwrap();
    let now = Arc::new(Mutex::new(start));
    let cache = DiskCache::open(Some(directory.path()), clock_at(Arc::clone(&now))).unwrap();
    (directory, cache, now)
}

fn object(field: &str, value: CacheValue) -> CacheValue {
    CacheValue::Object(BTreeMap::from([(field.to_owned(), value)]))
}

#[test]
fn disk_cache_reads_back_a_stored_value() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    let value = object("model", CacheValue::Text("gpt-4o".to_owned()));
    cache.set_cache("key", &value, None).unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), Some(value));
}

#[test]
fn disk_cache_reads_a_missing_key_as_none() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    assert_eq!(cache.get_cache("absent").unwrap(), None);
}

#[test]
fn disk_cache_creates_a_missing_directory() {
    let directory = TempDir::new().unwrap();
    let nested = directory.path().join("one").join("two");
    let cache = DiskCache::open(Some(&nested), Arc::new(|| 0.0_f64)).unwrap();
    cache
        .set_cache("key", &CacheValue::Integer(1), None)
        .unwrap();
    assert_eq!(
        cache.get_cache("key").unwrap(),
        Some(CacheValue::Integer(1))
    );
}

#[test]
fn disk_cache_defaults_to_the_dot_litellm_cache_directory() {
    let directory = TempDir::new().unwrap();
    let previous = current_dir().unwrap();
    set_current_dir(directory.path()).unwrap();
    let cache = DiskCache::open(None, Arc::new(|| 1000.0_f64)).unwrap();
    cache
        .set_cache("key", &CacheValue::Integer(1), None)
        .unwrap();
    let created = Path::new(DEFAULT_DISK_CACHE_DIRECTORY).join("cache.db");
    assert!(created.exists(), "{} is missing", created.display());
    set_current_dir(previous).unwrap();
}

#[test]
fn disk_cache_keeps_entries_across_reopens() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().to_owned();
    let cache = DiskCache::open(Some(&path), Arc::new(|| 10.0_f64)).unwrap();
    cache
        .set_cache("key", &CacheValue::Text("kept".to_owned()), None)
        .unwrap();
    drop(cache);
    let reopened = DiskCache::open(Some(Path::new(&path)), Arc::new(|| 20.0_f64)).unwrap();
    assert_eq!(
        reopened.get_cache("key").unwrap(),
        Some(CacheValue::Text("kept".to_owned()))
    );
}

#[test]
fn disk_cache_expires_a_value_at_its_ttl() {
    let (_directory, cache, now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), Some(30.0_f64))
        .unwrap();
    *now.lock().unwrap() = 1030.0_f64;
    assert_eq!(cache.get_cache("key").unwrap(), None);
}

#[test]
fn disk_cache_keeps_a_value_just_before_its_ttl() {
    let (_directory, cache, now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), Some(30.0_f64))
        .unwrap();
    *now.lock().unwrap() = 1029.999_f64;
    assert_eq!(
        cache.get_cache("key").unwrap(),
        Some(CacheValue::Integer(1))
    );
}

#[test]
fn disk_cache_keeps_a_value_without_a_ttl_forever() {
    let (_directory, cache, now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), None)
        .unwrap();
    *now.lock().unwrap() = 1_000_000.0_f64;
    assert_eq!(
        cache.get_cache("key").unwrap(),
        Some(CacheValue::Integer(1))
    );
}

#[rstest]
#[case::null(CacheValue::Null)]
#[case::false_value(CacheValue::Boolean(false))]
#[case::zero(CacheValue::Integer(0))]
#[case::float_zero(CacheValue::Float(0.0))]
#[case::negative_float_zero(CacheValue::Float(-0.0))]
#[case::empty_text(CacheValue::Text(String::new()))]
#[case::empty_array(CacheValue::Array(Vec::new()))]
#[case::empty_object(CacheValue::Object(BTreeMap::new()))]
fn disk_cache_reads_a_falsy_entry_as_a_miss(#[case] value: CacheValue) {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache.set_cache("key", &value, None).unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), None);
}

#[rstest]
#[case::true_value(CacheValue::Boolean(true))]
#[case::one(CacheValue::Integer(1))]
#[case::negative_number(CacheValue::Integer(-1))]
#[case::float(CacheValue::Float(2.5))]
#[case::text_zero(CacheValue::Text("0".to_owned()))]
#[case::one_element_array(CacheValue::Array(vec![CacheValue::Null]))]
#[case::one_field_object(object("model", CacheValue::Integer(0)))]
fn disk_cache_reads_a_truthy_entry_back(#[case] value: CacheValue) {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache.set_cache("key", &value, None).unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), Some(value));
}

// LiteLLM json.loads a stored string, so a text payload that looks like json reads back as its decoded value there
#[test]
fn disk_cache_keeps_a_json_looking_text_as_text() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    let value = CacheValue::Text("{\"model\": \"gpt-4o\"}".to_owned());
    cache.set_cache("key", &value, None).unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), Some(value));
}

#[test]
fn disk_cache_refuses_a_binary_value() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    let stored = cache.set_cache("key", &CacheValue::Bytes(vec![1, 2]), None);
    assert!(matches!(stored, Err(DiskError::Serialize { .. })));
}

#[test]
fn disk_cache_keeps_the_batch_order() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("first", &CacheValue::Integer(1), None)
        .unwrap();
    cache
        .set_cache("third", &CacheValue::Integer(3), None)
        .unwrap();
    let keys = ["first", "missing", "third"].map(str::to_owned);
    let values = cache.batch_get_cache(&keys).unwrap();
    assert_eq!(
        values,
        vec![
            Some(CacheValue::Integer(1)),
            None,
            Some(CacheValue::Integer(3))
        ]
    );
}

#[test]
fn disk_cache_batch_reads_a_falsy_entry_as_a_miss() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("zero", &CacheValue::Integer(0), None)
        .unwrap();
    let values = cache.batch_get_cache(&["zero".to_owned()]).unwrap();
    assert_eq!(values, vec![None]);
}

#[test]
fn disk_cache_increments_from_the_stored_integer() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("counter", &CacheValue::Integer(2), None)
        .unwrap();
    assert_eq!(cache.increment_cache("counter", 3, None).unwrap(), 5);
    assert_eq!(
        cache.get_cache("counter").unwrap(),
        Some(CacheValue::Integer(5))
    );
}

fn increment_twenty_five_times(path: &Path) {
    let handle = DiskCache::open(Some(path), Arc::new(|| 1000.0_f64)).unwrap();
    for _ in 0..25_usize {
        handle.increment_cache("counter", 1, None).unwrap();
    }
}

fn increment_from_two_handles(path: &Path) {
    scope(|scope| {
        for _ in 0..2_usize {
            let worker_path = path.to_owned();
            scope.spawn(move || increment_twenty_five_times(&worker_path));
        }
    });
}

#[test]
fn disk_cache_increments_stay_atomic_across_two_handles() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().to_owned();
    let counter = DiskCache::open(Some(&path), Arc::new(|| 1000.0_f64)).unwrap();
    counter
        .set_cache("counter", &CacheValue::Integer(0), None)
        .unwrap();
    increment_from_two_handles(&path);
    assert_eq!(
        counter.get_cache("counter").unwrap(),
        Some(CacheValue::Integer(50))
    );
}

#[test]
fn disk_cache_increments_a_missing_key_from_zero() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    assert_eq!(cache.increment_cache("counter", 4, None).unwrap(), 4);
}

#[test]
fn disk_cache_increments_ignores_a_stored_float() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("counter", &CacheValue::Float(2.5), None)
        .unwrap();
    assert_eq!(cache.increment_cache("counter", 1, None).unwrap(), 1);
}

#[test]
fn disk_cache_increments_ignores_a_stored_text() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("counter", &CacheValue::Text("5".to_owned()), None)
        .unwrap();
    assert_eq!(cache.increment_cache("counter", 1, None).unwrap(), 1);
}

#[test]
fn disk_cache_increments_with_the_given_ttl() {
    let (_directory, cache, now) = temp_cache(1000.0);
    cache.increment_cache("counter", 1, Some(30.0_f64)).unwrap();
    *now.lock().unwrap() = 1030.0_f64;
    assert_eq!(cache.get_cache("counter").unwrap(), None);
}

#[test]
fn disk_cache_increment_refuses_an_overflow() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("counter", &CacheValue::Integer(i64::MAX), None)
        .unwrap();
    let incremented = cache.increment_cache("counter", 1, None);
    assert!(matches!(
        incremented,
        Err(DiskError::IncrementOutOfRange { .. })
    ));
}

#[test]
fn disk_cache_deletes_a_key() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), None)
        .unwrap();
    cache.delete_cache("key").unwrap();
    assert_eq!(cache.get_cache("key").unwrap(), None);
}

#[test]
fn disk_cache_delete_of_a_missing_key_is_a_no_op() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache.delete_cache("absent").unwrap();
}

#[test]
fn disk_cache_delete_keeps_an_unrelated_key() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("kept", &CacheValue::Integer(2), None)
        .unwrap();
    cache.delete_cache("absent").unwrap();
    assert_eq!(
        cache.get_cache("kept").unwrap(),
        Some(CacheValue::Integer(2))
    );
}

#[test]
fn disk_cache_delete_of_an_expired_key_is_a_no_op() {
    let (_directory, cache, now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), Some(5.0_f64))
        .unwrap();
    *now.lock().unwrap() = 1100.0_f64;
    cache.delete_cache("key").unwrap();
}

#[test]
fn disk_cache_flush_removes_every_key() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("first", &CacheValue::Integer(1), None)
        .unwrap();
    cache
        .set_cache("second", &CacheValue::Integer(2), None)
        .unwrap();
    cache.flush_cache().unwrap();
    assert_eq!(cache.get_cache("first").unwrap(), None);
    assert_eq!(cache.get_cache("second").unwrap(), None);
}

#[test]
fn disk_cache_rejects_a_non_finite_clock() {
    let directory = TempDir::new().unwrap();
    let cache = DiskCache::open(Some(directory.path()), Arc::new(|| f64::NAN)).unwrap();
    assert!(matches!(
        cache.get_cache("key"),
        Err(DiskError::InvalidClock)
    ));
    assert!(matches!(
        cache.set_cache("key", &CacheValue::Integer(1), None),
        Err(DiskError::InvalidClock)
    ));
}

#[test]
fn disk_cache_reports_a_corrupt_stored_body() {
    let (directory, cache, _now) = temp_cache(1000.0);
    cache
        .set_cache("key", &CacheValue::Integer(1), None)
        .unwrap();
    let connection = Connection::open(directory.path().join("cache.db")).unwrap();
    connection
        .execute("UPDATE cache SET body = 'not json' WHERE key = 'key'", [])
        .unwrap();
    assert!(matches!(
        cache.get_cache("key"),
        Err(DiskError::Decode { .. })
    ));
}

#[tokio::test]
async fn disk_cache_async_calls_match_the_sync_ones() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .async_set_cache("key", CacheValue::Integer(2), None)
        .await
        .unwrap();
    assert_eq!(
        cache.async_get_cache("key").await.unwrap(),
        Some(CacheValue::Integer(2))
    );
    let keys = ["key".to_owned(), "absent".to_owned()];
    assert_eq!(
        cache.async_batch_get_cache(&keys).await.unwrap(),
        vec![Some(CacheValue::Integer(2)), None]
    );
    assert_eq!(cache.async_increment("key", 3, None).await.unwrap(), 5);
    cache.async_delete_cache("key").await.unwrap();
    assert_eq!(cache.async_get_cache("key").await.unwrap(), None);
    cache
        .async_set_cache("key", CacheValue::Integer(1), None)
        .await
        .unwrap();
    cache.async_flush_cache().await.unwrap();
    assert_eq!(cache.async_get_cache("key").await.unwrap(), None);
}

#[tokio::test]
async fn disk_cache_async_pipeline_writes_every_entry() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    let entries = vec![
        ("first".to_owned(), CacheValue::Integer(1)),
        ("second".to_owned(), CacheValue::Integer(2)),
    ];
    cache.async_set_cache_pipeline(entries, None).await.unwrap();
    assert_eq!(
        cache.get_cache("first").unwrap(),
        Some(CacheValue::Integer(1))
    );
    assert_eq!(
        cache.get_cache("second").unwrap(),
        Some(CacheValue::Integer(2))
    );
}

#[tokio::test]
async fn disk_cache_async_pipeline_applies_the_ttl() {
    let (_directory, cache, now) = temp_cache(1000.0);
    let entries = vec![("piped".to_owned(), CacheValue::Integer(1))];
    cache
        .async_set_cache_pipeline(entries, Some(30.0_f64))
        .await
        .unwrap();
    *now.lock().unwrap() = 1030.0_f64;
    assert_eq!(cache.get_cache("piped").unwrap(), None);
}

#[tokio::test]
async fn disk_cache_async_reads_a_falsy_entry_as_a_miss() {
    let (_directory, cache, _now) = temp_cache(1000.0);
    cache
        .async_set_cache("key", CacheValue::Integer(0), None)
        .await
        .unwrap();
    assert_eq!(cache.async_get_cache("key").await.unwrap(), None);
}
