use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use indexmap::IndexMap;
use serde_json::Number;

use super::support::local_memory;
use crate::dual_cache::limited_size::LimitedSizeOrderedDict;
use crate::dual_cache::value::{cache_from_json, json_from_cached};
use crate::dual_cache::{
    BATCH_CACHE_SIZE_VARIABLE, CacheOptions, DEFAULT_BATCH_CACHE_EXPIRY,
    DEFAULT_MAX_SIZE_IN_MEMORY, DualCache, DualCacheError, DualCacheSettings,
    MAX_REDIS_BATCH_CACHE_SIZE, Ttl, lock_read, lock_write,
};
use crate::types::pipeline::RedisPipelineIncrement;

fn limited(max_size: usize) -> LimitedSizeOrderedDict<f64> {
    LimitedSizeOrderedDict {
        entries: IndexMap::new(),
        max_size,
    }
}

fn write(ttl: Option<f64>, local_only: bool) -> CacheOptions {
    CacheOptions {
        ttl,
        local_only,
        refresh_ttl: false,
    }
}

fn settings(default_in_memory_ttl: Option<f64>) -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn limited_size_tracks_insertion_order_and_evicts_the_oldest() {
    let mut dict = limited(2);
    dict.insert("first".to_owned(), 1.0_f64);
    dict.insert("second".to_owned(), 2.0_f64);
    dict.insert("third".to_owned(), 3.0_f64);
    assert_eq!(dict.entries.len(), 2);
    assert!(!dict.entries.contains_key("first"));
    assert_eq!(dict.entries.get("second"), Some(&2.0_f64));
    assert_eq!(dict.entries.shift_remove("second"), Some(2.0_f64));
    assert_eq!(dict.entries.get("third"), Some(&3.0_f64));
}

#[test]
fn limited_size_evicts_the_oldest_even_when_the_key_already_exists() {
    let mut dict = limited(2);
    dict.insert("first".to_owned(), 1.0_f64);
    dict.insert("second".to_owned(), 2.0_f64);
    dict.insert("second".to_owned(), 20.0_f64);
    assert_eq!(dict.entries.len(), 1);
    assert_eq!(dict.entries.get("first"), None);
    assert_eq!(dict.entries.get("second"), Some(&20.0_f64));
}

#[test]
fn limited_size_zero_tracks_nothing() {
    let mut dict = limited(0);
    dict.insert("key".to_owned(), 1.0_f64);
    assert_eq!(dict.entries.len(), 0);
    assert_eq!(dict.entries.get("key"), None);
}

#[test]
fn constructor_resolves_the_defaults() {
    let settings = DualCacheSettings::from_environment(&BTreeMap::new());
    let cache = DualCache::new(local_memory(1000.0), None, settings);
    assert_eq!(
        cache.redis_batch_cache_expiry.to_bits(),
        DEFAULT_BATCH_CACHE_EXPIRY.to_bits()
    );
    assert_eq!(
        cache.last_redis_batch_access_time.lock().unwrap().max_size,
        MAX_REDIS_BATCH_CACHE_SIZE
    );
    assert_eq!(cache.redis_delete_chunk_size, MAX_REDIS_BATCH_CACHE_SIZE);
    assert_eq!(*lock_read(&cache.default_in_memory_ttl), None);
    assert_eq!(*lock_read(&cache.default_redis_ttl), None);
}

#[test]
fn constructor_keeps_every_explicit_setting() {
    let cache = DualCache::new(
        local_memory(1000.0),
        None,
        DualCacheSettings {
            default_in_memory_ttl: Some(5.0_f64),
            default_redis_ttl: Some(7.0_f64),
            batch_cache_expiry: Some(0.5_f64),
            max_redis_batch_cache_size: 3,
            redis_delete_chunk_size: 4,
        },
    );
    assert_eq!(cache.redis_batch_cache_expiry.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        cache.last_redis_batch_access_time.lock().unwrap().max_size,
        3
    );
    assert_eq!(cache.redis_delete_chunk_size, 4);
    assert_eq!(*lock_read(&cache.default_in_memory_ttl), Some(5.0_f64));
    assert_eq!(*lock_read(&cache.default_redis_ttl), Some(7.0_f64));
}

fn sizes_for(value: Option<&str>) -> (usize, usize) {
    let environment: BTreeMap<String, String> = value
        .map(|text| (BATCH_CACHE_SIZE_VARIABLE.to_owned(), text.to_owned()))
        .into_iter()
        .collect();
    let resolved = DualCacheSettings::from_environment(&environment);
    (
        resolved.max_redis_batch_cache_size,
        resolved.redis_delete_chunk_size,
    )
}

#[test]
fn settings_from_environment_read_the_batch_size_variable() {
    assert_eq!(sizes_for(Some("500")), (500, 500));
    assert_eq!(sizes_for(Some(" 7 ")), (7, 7));
    assert_eq!(sizes_for(Some("1")), (1, 1));
}

#[test]
fn settings_from_environment_keep_the_default_for_an_unset_empty_or_unusable_value() {
    let default = (MAX_REDIS_BATCH_CACHE_SIZE, MAX_REDIS_BATCH_CACHE_SIZE);
    assert_eq!(sizes_for(None), default);
    assert_eq!(sizes_for(Some("")), default);
    assert_eq!(sizes_for(Some("many")), default);
    assert_eq!(sizes_for(Some("0")), default);
    assert_eq!(sizes_for(Some("-3")), default);
}

#[test]
fn update_cache_ttl_keeps_a_value_the_call_leaves_out() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(Some(5.0_f64)));
    *lock_write(&cache.default_redis_ttl) = Some(7.0_f64);
    cache.update_cache_ttl(None, Some(9.0_f64));
    assert_eq!(*lock_read(&cache.default_in_memory_ttl), Some(5.0_f64));
    assert_eq!(*lock_read(&cache.default_redis_ttl), Some(9.0_f64));
    cache.update_cache_ttl(Some(11.0_f64), None);
    assert_eq!(*lock_read(&cache.default_in_memory_ttl), Some(11.0_f64));
    assert_eq!(*lock_read(&cache.default_redis_ttl), Some(9.0_f64));
}

#[test]
fn update_in_memory_max_size_evicts_at_the_new_size_on_the_next_set() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.update_in_memory_max_size(Some(1));
    cache.set_cache("first", &text("one"), write(None, true));
    assert_eq!(cache.get_cache("first", true), Some(text("one")));
    cache.set_cache("second", &text("two"), write(None, true));
    assert_eq!(cache.get_cache("first", true), None);
    assert_eq!(cache.get_cache("second", true), Some(text("two")));
}

#[test]
fn update_in_memory_max_size_zero_stores_nothing() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.update_in_memory_max_size(Some(0));
    cache.set_cache("key", &text("value"), write(None, true));
    assert_eq!(cache.get_cache("key", true), None);
    let counted = cache.increment_cache("count", 1.0_f64, write(None, true));
    assert_eq!(counted.expect("the increment").to_bits(), 1.0_f64.to_bits());
    assert_eq!(cache.get_cache("count", true), None);
}

#[tokio::test]
async fn update_in_memory_max_size_restores_the_default_for_none() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.update_in_memory_max_size(Some(1));
    cache.update_in_memory_max_size(None);
    let entries: Vec<(String, Json)> = (0..=DEFAULT_MAX_SIZE_IN_MEMORY)
        .map(|index| (format!("key-{index:03}"), text("value")))
        .collect();
    cache
        .async_set_cache_pipeline(&entries, write(None, true))
        .await;
    assert_eq!(cache.get_cache("key-000", true), None);
    assert_eq!(cache.get_cache("key-001", true), Some(text("value")));
    assert_eq!(cache.get_cache("key-200", true), Some(text("value")));
}

#[tokio::test]
async fn set_cache_applies_the_default_memory_ttl() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(Some(30.0_f64)));
    cache.set_cache("key", &text("value"), write(None, false));
    let ttl = cache.async_get_ttl("key").await.expect("the memory ttl");
    assert_eq!(ttl, Some(Ttl::Expiry(1030.0_f64)));
}

#[tokio::test]
async fn set_cache_keeps_an_explicit_ttl_over_the_default() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(Some(30.0_f64)));
    cache.set_cache("key", &text("value"), write(Some(5.0_f64), false));
    let ttl = cache.async_get_ttl("key").await.expect("the memory ttl");
    assert_eq!(ttl, Some(Ttl::Expiry(1005.0_f64)));
}

#[tokio::test]
async fn get_cache_returns_a_memory_hit() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("key", &text("value"), write(None, false));
    assert_eq!(cache.get_cache("key", false), Some(text("value")));
    assert_eq!(
        cache.async_get_cache("key", false).await,
        Some(text("value"))
    );
}

#[tokio::test]
async fn get_cache_treats_a_stored_null_as_a_miss() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("key", &Json::Null, write(None, false));
    assert_eq!(cache.get_cache("key", false), None);
    assert_eq!(cache.async_get_cache("key", false).await, None);
}

#[tokio::test]
async fn increment_cache_returns_the_memory_result_without_redis() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    let result = cache
        .increment_cache("key", 2.0_f64, write(None, false))
        .expect("the memory increment");
    assert_eq!(result.to_bits(), 2.0_f64.to_bits());
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::Number(
            Number::from_f64(2.0_f64).expect("a finite number")
        ))
    );
}

#[tokio::test]
async fn async_increment_cache_pipeline_returns_the_memory_results() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    let increments = [
        RedisPipelineIncrement {
            key: "first".to_owned(),
            increment_value: 1.0_f64,
            ttl: None,
        },
        RedisPipelineIncrement {
            key: "second".to_owned(),
            increment_value: 2.5_f64,
            ttl: None,
        },
    ];
    let results = cache
        .async_increment_cache_pipeline(&increments, false)
        .await
        .expect("the memory pipeline");
    assert_eq!(results, vec![1.0_f64, 2.5_f64]);
}

#[tokio::test]
async fn async_set_cache_sadd_unions_the_memory_members() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache
        .async_set_cache_sadd("key", &["a".to_owned(), "b".to_owned()], write(None, false))
        .await
        .expect("the first set add");
    cache
        .async_set_cache_sadd("key", &["b".to_owned(), "c".to_owned()], write(None, false))
        .await
        .expect("the second set add");
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::List(vec![text("a"), text("b"), text("c")]))
    );
}

#[tokio::test]
async fn async_set_cache_sadd_rejects_a_value_that_is_not_a_set() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("key", &text("not a set"), write(None, false));
    let error = cache
        .async_set_cache_sadd("key", &["a".to_owned()], write(None, true))
        .await
        .expect_err("a string is not a set");
    assert_eq!(
        error.to_string(),
        "the value under key is not a set of strings"
    );
}

#[tokio::test]
async fn async_delete_cache_keys_removes_every_memory_key_without_redis() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("kept", &text("value"), write(None, false));
    cache.set_cache("first", &text("value"), write(None, false));
    cache.set_cache("second", &text("value"), write(None, false));
    cache
        .async_delete_cache_keys(&["first".to_owned(), "second".to_owned()])
        .await
        .expect("the memory deletion");
    assert_eq!(cache.get_cache("first", true), None);
    assert_eq!(cache.get_cache("second", true), None);
    assert_eq!(cache.get_cache("kept", true), Some(text("value")));
}

#[tokio::test]
async fn async_delete_cache_keys_ignores_an_empty_list() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("kept", &text("value"), write(None, false));
    cache
        .async_delete_cache_keys(&[])
        .await
        .expect("an empty deletion");
    assert_eq!(cache.get_cache("kept", true), Some(text("value")));
}

#[tokio::test]
async fn flush_cache_and_delete_cache_clear_the_memory_tier() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("first", &text("value"), write(None, false));
    cache.set_cache("second", &text("value"), write(None, false));
    cache.delete_cache("first").expect("the memory deletion");
    assert_eq!(cache.get_cache("first", true), None);
    assert_eq!(cache.get_cache("second", true), Some(text("value")));
    cache.flush_cache().expect("the memory flush");
    assert_eq!(cache.get_cache("second", true), None);
}

#[tokio::test]
async fn async_get_ttl_reports_the_memory_expiry_before_redis() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.set_cache("key", &text("value"), write(Some(7.0_f64), false));
    assert_eq!(
        cache.async_get_ttl("key").await.expect("the memory ttl"),
        Some(Ttl::Expiry(1007.0_f64))
    );
    assert_eq!(cache.async_get_ttl("absent").await.expect("a miss"), None);
}

#[tokio::test]
async fn pre_call_operations_return_none_without_redis() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(Some(30.0_f64)));
    assert!(
        cache
            .set_cache_pre_call("key", &text("value"), None)
            .expect("the pre-call set")
            .is_none()
    );
    assert!(
        cache
            .delete_cache_pre_call("key")
            .expect("the pre-call delete")
            .is_none()
    );
    assert_eq!(cache.get_cache("key", true), None);
}

#[tokio::test]
async fn pipeline_increment_post_call_without_a_batch_takes_the_direct_path() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    let increments = [RedisPipelineIncrement {
        key: "key".to_owned(),
        increment_value: 3.0_f64,
        ttl: None,
    }];
    cache.pipeline_increment_post_call(&increments).await;
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::Number(
            Number::from_f64(3.0_f64).expect("a finite number")
        ))
    );
}

#[tokio::test]
async fn increment_cache_post_call_without_a_batch_increments_memory() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    cache.increment_cache_post_call("key", 4.0_f64, None).await;
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::Number(
            Number::from_f64(4.0_f64).expect("a finite number")
        ))
    );
}

#[tokio::test]
async fn set_cache_refuses_an_integer_the_memory_tier_cannot_hold() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    let too_large = Json::Number(Number::from(u64::MAX));
    cache.set_cache("key", &too_large, write(None, false));
    assert_eq!(cache.get_cache("key", true), None);
}

#[tokio::test]
async fn async_increment_cache_honours_local_only() {
    let cache = DualCache::new(local_memory(1000.0), None, settings(None));
    assert_eq!(
        cache
            .async_increment_cache("key", 2.0_f64, write(None, true))
            .await,
        Some(2.0_f64)
    );
}

fn number_marker(extra: Option<(&str, CachedValue)>, digits: &str) -> CachedValue {
    let mut fields = BTreeMap::from([(
        "$serde_json::private::Number".to_owned(),
        CachedValue::Text(digits.to_owned()),
    )]);
    fields.extend(extra.map(|(key, entry)| (key.to_owned(), entry)));
    CachedValue::Object(fields)
}

#[test]
fn a_decoded_float_marker_becomes_a_number() {
    let number = Number::from_f64(12.5).expect("a finite float");
    assert_eq!(
        json_from_cached(number_marker(None, "12.5")),
        Json::Number(number)
    );
}

#[test]
fn a_marker_with_a_sibling_or_without_digits_stays_an_object() {
    let sibling = json_from_cached(number_marker(Some(("other", CachedValue::Null)), "1"));
    let Json::Object(fields) = sibling else {
        panic!("a marker with a sibling is a plain object");
    };
    assert_eq!(fields.len(), 2);
    let broken = json_from_cached(number_marker(None, "not a number"));
    assert!(matches!(broken, Json::Object(_)));
}

#[test]
fn cache_from_json_keeps_a_nested_float_a_float() {
    let spend = Number::from_f64(12.5).expect("a finite float");
    let value = Json::List(vec![Json::Object(IndexMap::from([(
        "spend".to_owned(),
        Json::Number(spend),
    )]))]);
    let stored = cache_from_json(&value).expect("a storable value");
    let CacheValue::Array(items) = stored else {
        panic!("a list stores as an array");
    };
    let CacheValue::Object(fields) = &items[0] else {
        panic!("an object stores as an object");
    };
    assert_eq!(fields["spend"], CacheValue::Float(12.5));
}

#[test]
fn cache_from_json_stores_whole_numbers_as_integers_and_refuses_a_wide_one() {
    let whole = Json::Number(Number::from(7_i64));
    assert_eq!(
        cache_from_json(&whole).expect("an integer"),
        CacheValue::Integer(7)
    );
    let wide = Json::Number(Number::from(u64::MAX));
    assert!(matches!(
        cache_from_json(&wide),
        Err(DualCacheError::UnrepresentableValue)
    ));
}
