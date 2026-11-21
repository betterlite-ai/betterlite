use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_cache::{WriteOptions, gateway_cache};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_models::decisions::Json;

use crate::cleanup_schedule::cache_size::{CACHE_MAX_SIZE_KEY, on_cache_size_changed};
use crate::cleanup_schedule::ports::CacheMaxSize;

use super::support::{self, capture_logs};

#[derive(Default)]
struct RecordingCache {
    sizes: Mutex<Vec<Option<usize>>>,
}

impl RecordingCache {
    fn sizes(&self) -> Vec<Option<usize>> {
        self.sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CacheMaxSize for RecordingCache {
    fn resize(&self, max_size: Option<usize>) {
        self.sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(max_size);
    }
}

fn db_row(value: JsonValue) -> BTreeMap<String, JsonValue> {
    BTreeMap::from([(CACHE_MAX_SIZE_KEY.to_owned(), value)])
}

fn applied_store(value: JsonValue) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.apply_db_row(DbRow::GeneralSettings, db_row(value));
    store
}

#[test]
fn the_auth_cache_port_applies_the_max_size_to_the_memory_tier() {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1000.0_f64);
    let memory = Arc::new(LocalMemoryOps::new(1024, clock).unwrap());
    let cache = gateway_cache(
        Arc::clone(&memory),
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: Some(60.0_f64),
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    );
    CacheMaxSize::resize(&cache, Some(1));
    let options = WriteOptions {
        ttl: None,
        local_only: true,
        refresh_ttl: false,
    };
    cache
        .set(Some("tag:first"), &Json::Text("first".to_owned()), options)
        .unwrap();
    cache
        .set(
            Some("tag:second"),
            &Json::Text("second".to_owned()),
            options,
        )
        .unwrap();
    assert_eq!(cache.get("tag:first", true), None);
    assert_eq!(
        cache.get("tag:second", true),
        Some(Json::Text("second".to_owned()))
    );
}

#[test]
fn the_cache_hook_ignores_a_key_the_database_never_set() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    let cache = RecordingCache::default();
    on_cache_size_changed(&BTreeMap::new(), false, &mut store, &cache).unwrap();
    assert!(cache.sizes().is_empty());
}

#[test]
fn a_valid_database_size_is_written_back_and_applied() {
    let mut store = applied_store(JsonValue::from(500_i64));
    let cache = RecordingCache::default();
    let values = db_row(JsonValue::from(500_i64));
    on_cache_size_changed(&values, false, &mut store, &cache).unwrap();
    assert_eq!(cache.sizes(), vec![Some(500_usize)]);
    assert_eq!(
        store.get(CACHE_MAX_SIZE_KEY),
        Some(&JsonValue::from(500_u64))
    );
}

#[test]
fn an_integral_string_size_is_accepted_and_stored_as_a_number() {
    let mut store = applied_store(JsonValue::from("250"));
    let cache = RecordingCache::default();
    let values = db_row(JsonValue::from("250"));
    on_cache_size_changed(&values, false, &mut store, &cache).unwrap();
    assert_eq!(cache.sizes(), vec![Some(250_usize)]);
    assert_eq!(
        store.get(CACHE_MAX_SIZE_KEY),
        Some(&JsonValue::from(250_u64))
    );
}

#[test]
fn an_invalid_database_size_is_removed_and_the_cache_is_untouched() {
    let (buffer, _guard) = capture_logs();
    for (value, shown) in [
        (JsonValue::from(0_i64), "=0 "),
        (JsonValue::from(-3_i64), "=-3 "),
        (JsonValue::from(2.5_f64), "=2.5 "),
        (JsonValue::from("soon"), "=\"soon\" "),
        (JsonValue::from(true), "=true "),
    ] {
        let mut store = applied_store(value.clone());
        let cache = RecordingCache::default();
        let values = db_row(value);
        on_cache_size_changed(&values, false, &mut store, &cache).unwrap();
        assert!(cache.sizes().is_empty(), "{shown}");
        assert_eq!(store.get(CACHE_MAX_SIZE_KEY), None, "{shown}");
        assert!(
            buffer.contents().contains(&format!(
                "Ignoring invalid general_settings.user_api_key_cache_max_size{shown}from the DB"
            )),
            "{shown}"
        );
    }
}

#[test]
fn a_removed_database_value_resets_the_cache_size() {
    let mut store = applied_store(JsonValue::from(500_i64));
    store.apply_db_row(DbRow::GeneralSettings, BTreeMap::new());
    let cache = RecordingCache::default();
    on_cache_size_changed(&BTreeMap::new(), true, &mut store, &cache).unwrap();
    assert_eq!(cache.sizes(), vec![None]);
    assert_eq!(store.get(CACHE_MAX_SIZE_KEY), None);
}

#[test]
fn a_null_database_value_resets_the_cache_size() {
    let mut store = applied_store(JsonValue::Null);
    let cache = RecordingCache::default();
    on_cache_size_changed(&db_row(JsonValue::Null), false, &mut store, &cache).unwrap();
    assert_eq!(cache.sizes(), vec![None]);
    assert_eq!(store.get(CACHE_MAX_SIZE_KEY), None);
}

#[test]
fn a_config_value_wins_over_an_invalid_database_value() {
    let mut store = support::store(&[(CACHE_MAX_SIZE_KEY, JsonValue::from(400_i64))]);
    store.apply_db_row(DbRow::GeneralSettings, db_row(JsonValue::from(0_i64)));
    let cache = RecordingCache::default();
    on_cache_size_changed(&db_row(JsonValue::from(0_i64)), false, &mut store, &cache).unwrap();
    assert_eq!(cache.sizes(), vec![Some(400_usize)]);
    assert_eq!(
        store.get(CACHE_MAX_SIZE_KEY),
        Some(&JsonValue::from(400_i64))
    );
}

#[test]
fn a_config_owned_invalid_value_is_never_removed() {
    let (buffer, _guard) = capture_logs();
    let mut store = support::store(&[(CACHE_MAX_SIZE_KEY, JsonValue::from(0_i64))]);
    store.apply_db_row(DbRow::GeneralSettings, db_row(JsonValue::from(0_i64)));
    let cache = RecordingCache::default();
    on_cache_size_changed(&db_row(JsonValue::from(0_i64)), false, &mut store, &cache).unwrap();
    assert!(cache.sizes().is_empty());
    assert_eq!(store.get(CACHE_MAX_SIZE_KEY), Some(&JsonValue::from(0_i64)));
    assert!(
        buffer.contents().contains(
            "Ignoring invalid general_settings.user_api_key_cache_max_size=0 from the DB"
        )
    );
}
