#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::*;
use crate::cache_backends::settings::{DiskSettings, DualSettings, MemorySettings, RedisSettings};

#[test]
fn backends_request_settings_reads_absent_keys_as_none() {
    let settings: MemorySettings = parse_settings("{}").unwrap();
    assert_eq!(settings.max_size_in_memory, None);
    assert_eq!(settings.max_size_per_item, None);
}

#[test]
fn backends_request_settings_keeps_an_explicit_zero() {
    let settings: MemorySettings = parse_settings(r#"{"max_size_in_memory": 0}"#).unwrap();
    assert_eq!(settings.max_size_in_memory, Some(0));
}

#[test]
fn backends_request_settings_rejects_an_unknown_key() {
    parse_settings::<MemorySettings>(r#"{"nope": 1}"#).unwrap_err();
}

#[test]
fn backends_request_settings_rejects_malformed_text() {
    parse_settings::<MemorySettings>("{").unwrap_err();
}

#[test]
fn backends_request_settings_reads_the_disk_directory() {
    let settings: DiskSettings = parse_settings(r#"{"disk_cache_dir": "/tmp/cache"}"#).unwrap();
    assert_eq!(
        settings.disk_cache_dir.as_deref(),
        Some(Path::new("/tmp/cache"))
    );
}

#[test]
fn backends_request_settings_reads_the_redis_overrides() {
    let settings: RedisSettings = parse_settings(
        r#"{"namespace": "ns", "default_ttl": 60, "overrides": {"host": "127.0.0.1"}}"#,
    )
    .unwrap();
    assert_eq!(settings.namespace.as_deref(), Some("ns"));
    assert_eq!(settings.default_ttl, Some(60));
}

#[test]
fn backends_request_settings_reads_the_dual_defaults() {
    let settings: DualSettings =
        parse_settings(r#"{"default_in_memory_ttl": 30, "max_redis_batch_cache_size": 5}"#)
            .unwrap();
    assert_eq!(settings.default_in_memory_ttl, Some(30.0_f64));
    assert_eq!(settings.max_redis_batch_cache_size, Some(5_usize));
}

#[test]
fn backends_request_integer_value_keeps_its_whole_form() {
    assert_eq!(cache_value("2").unwrap(), CacheValue::Integer(2));
}

#[test]
fn backends_request_float_value_stays_a_float() {
    assert_eq!(cache_value("1.5").unwrap(), CacheValue::Float(1.5_f64));
}

#[test]
fn backends_request_object_value_keeps_its_keys() {
    let value = cache_value(r#"{"a": true, "b": null}"#).unwrap();
    let CacheValue::Object(fields) = value else {
        panic!("the object read as another variant");
    };
    assert_eq!(fields.get("a"), Some(&CacheValue::Boolean(true)));
    assert_eq!(fields.get("b"), Some(&CacheValue::Null));
}

#[test]
fn backends_request_malformed_value_is_an_error() {
    cache_value("{").unwrap_err();
}

#[test]
fn backends_request_nesting_at_the_limit_parses() {
    let lists = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    json_value(&lists).unwrap();
    let objects = format!("{}1{}", r#"{"a":"#.repeat(MAX_DEPTH), "}".repeat(MAX_DEPTH));
    json_value(&objects).unwrap();
}

#[rstest]
#[case::lists_past_limit("[", "", "]", 129_usize)]
#[case::objects_past_limit(r#"{"a":"#, "1", "}", 129_usize)]
#[case::very_deep_lists("[", "", "]", 100_000_usize)]
fn backends_request_nesting_past_the_limit_is_an_error(
    #[case] opening: &str,
    #[case] value: &str,
    #[case] closing: &str,
    #[case] depth: usize,
) {
    let text = format!("{}{value}{}", opening.repeat(depth), closing.repeat(depth));
    let error = json_value(&text).unwrap_err();
    assert!(matches!(
        error,
        BackendHandleError::TooDeep { limit: MAX_DEPTH }
    ));
    assert!(error.is_value_error());
}

#[test]
fn backends_result_value_text_is_json() {
    let mut fields = BTreeMap::new();
    fields.insert("a".to_owned(), CacheValue::Integer(1));
    assert_eq!(
        text_of_value(&CacheValue::Object(fields)).unwrap(),
        r#"{"a":1}"#
    );
}

#[test]
fn backends_result_number_text_keeps_the_fraction() {
    assert_eq!(number_text(0.5_f64).unwrap(), "0.5");
}

#[test]
fn backends_result_cached_null_is_json_null() {
    assert_eq!(text_of_cached(CachedValue::Null).unwrap(), "null");
}

#[test]
fn backends_result_cached_number_is_json() {
    let value = CachedValue::Number(Number::from(3_i32));
    assert_eq!(text_of_cached(value).unwrap(), "3");
}

#[test]
fn backends_result_cached_float_reads_the_private_number_key() {
    let mut fields = BTreeMap::new();
    fields.insert(NUMBER_KEY.to_owned(), CachedValue::Text("1.5".to_owned()));
    assert_eq!(text_of_cached(CachedValue::Object(fields)).unwrap(), "1.5");
}

#[test]
fn backends_result_cached_object_keeps_its_keys() {
    let mut fields = BTreeMap::new();
    fields.insert("a".to_owned(), CachedValue::Boolean(true));
    assert_eq!(
        text_of_cached(CachedValue::Object(fields)).unwrap(),
        r#"{"a":true}"#
    );
}

#[test]
fn backends_result_json_round_trips_a_list() {
    let items = vec![Json::Text("x".to_owned()), Json::Null];
    let text = text_of_json(&Json::List(items)).unwrap();
    assert_eq!(text, r#"["x",null]"#);
    let Json::List(restored) = json_value(&text).unwrap() else {
        panic!("the list read as another variant");
    };
    assert_eq!(restored.len(), 2);
}

#[test]
fn backends_result_whole_seconds_truncates_the_fraction() {
    assert_eq!(whole_seconds(2.9_f64).unwrap(), 2);
}

#[test]
fn backends_result_whole_seconds_rejects_a_non_finite_value() {
    whole_seconds(f64::NAN).unwrap_err();
}

#[test]
fn backends_request_object_number_key_is_left_alone() {
    let mut object = IndexMap::new();
    object.insert(NUMBER_KEY.to_owned(), Json::Text("1.5".to_owned()));
    let CacheValue::Object(fields) =
        cache_value(&text_of_json(&Json::Object(object)).unwrap()).unwrap()
    else {
        panic!("the object read as another variant");
    };
    assert_eq!(
        fields.get(NUMBER_KEY),
        Some(&CacheValue::Text("1.5".to_owned()))
    );
}

#[test]
fn backends_request_nested_objects_keep_number_keys() {
    let text = r#" {"nested":[{"$serde_json::private::Number":"1.5"},1.5,{},[],true,null,"x"]} "#;
    let parsed = json_value(text).unwrap();
    assert_eq!(text_of_json(&parsed).unwrap(), text.trim());
    let nested = cache_value(text).unwrap();
    assert_eq!(text_of_value(&nested).unwrap(), text.trim());
    json_value("[1.5,").unwrap_err();
}

#[test]
fn backends_request_wall_clock() {
    assert!(wall_clock() > 1_700_000_000.0_f64);
}

#[test]
fn backends_result_cached_number_key_needs_one_field() {
    let mut fields = BTreeMap::new();
    fields.insert(NUMBER_KEY.to_owned(), CachedValue::Text("1.5".to_owned()));
    fields.insert("other".to_owned(), CachedValue::Boolean(true));
    assert_eq!(
        text_of_cached(CachedValue::Object(fields)).unwrap(),
        r#"{"$serde_json::private::Number":"1.5","other":true}"#
    );
}

#[test]
fn backends_result_cached_number_key_needs_digits() {
    let mut fields = BTreeMap::new();
    fields.insert(NUMBER_KEY.to_owned(), CachedValue::Text("xyz".to_owned()));
    assert_eq!(
        text_of_cached(CachedValue::Object(fields)).unwrap(),
        r#"{"$serde_json::private::Number":"xyz"}"#
    );
}
