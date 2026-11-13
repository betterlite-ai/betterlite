#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::sync::{Arc, Mutex};

use rstest::rstest;
use serde_json::Number;

use betterlite_models::decisions::{Extra, Json};

use crate::backend::CacheBackend;
use crate::cache_key::derive::get_cache_key;
use crate::cache_key::request::{LiteLlmParams, RequestCacheControl};
use crate::codec::CacheCodec;
use crate::embedding::embedding_cache::pipeline::async_add_cache_pipeline;
use crate::embedding::embedding_cache::{
    EmbeddingCacheError, add_embedding_entry_to_cache, convert_to_cached_embedding, tokens,
};
use crate::embedding::response::EmbeddingRecord;
use crate::types::cache_type::CacheMode;
use crate::types::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION, EmbeddingPayload};
use support::{
    RecordingCache, RecordingNative, RecordingStore, StoringBackend, TestCache, assert_envelope,
    cache, cached, decimal, details, inputs, integer, param, request, response, settings, text,
};

#[test]
fn convert_to_cached_embedding_builds_version_two_record() {
    let record = EmbeddingRecord {
        embedding: Some(EmbeddingPayload::Text("base64".to_owned())),
        index: Some(3),
        object: Some("embedding".to_owned()),
    };
    let prompt_details = details(&[("image_tokens", integer(2))]);
    let converted = convert_to_cached_embedding(
        &record,
        Some("text-embedding-3-small"),
        Some(4),
        Some(prompt_details.clone()),
    );
    assert_eq!(converted.embedding, record.embedding);
    assert_eq!(converted.index, Some(3));
    assert_eq!(converted.object.as_deref(), Some("embedding"));
    assert_eq!(converted.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(converted.prompt_tokens, Some(4));
    assert_eq!(converted.prompt_tokens_details, Some(prompt_details));
    assert_eq!(converted.format_version, 2);
    assert_eq!(converted.format_version, EMBEDDING_CACHE_FORMAT_VERSION);
}

#[test]
fn convert_to_cached_embedding_keeps_absent_fields_none() {
    let record = EmbeddingRecord {
        embedding: None,
        index: None,
        object: None,
    };
    let converted = convert_to_cached_embedding(&record, None, None, None);
    assert_eq!(
        converted,
        CachedEmbedding {
            embedding: None,
            index: None,
            object: None,
            model: None,
            prompt_tokens: None,
            prompt_tokens_details: None,
            format_version: EMBEDDING_CACHE_FORMAT_VERSION,
        }
    );
}

#[rstest]
#[case::single_item_keeps_total(1, 0, 10, 10)]
#[case::even_division(2, 0, 10, 5)]
#[case::even_division_late_index(2, 1, 10, 5)]
#[case::remainder_goes_to_earliest(3, 0, 10, 4)]
#[case::remainder_skips_late_indices(3, 1, 10, 3)]
#[case::remainder_last_index(3, 2, 10, 3)]
#[case::empty_data_keeps_total(0, 0, 10, 10)]
fn per_item_prompt_tokens_distributes(
    #[case] count: usize,
    #[case] idx: usize,
    #[case] total: i64,
    #[case] expected: i64,
) {
    let result = response(count, Some(total), None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, idx), Some(expected));
}

#[test]
fn per_item_prompt_tokens_without_usage_is_none() {
    let mut result = response(2, Some(10), None);
    result.usage = None;
    assert_eq!(tokens::per_item_prompt_tokens(&result, 0), None);
}

#[test]
fn per_item_prompt_tokens_without_total_is_none() {
    let result = response(2, None, None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, 0), None);
}

#[rstest]
#[case::first_index(0, -2)]
#[case::second_index(1, -3)]
fn per_item_prompt_tokens_floors_like_python(#[case] idx: usize, #[case] expected: i64) {
    let result = response(2, Some(-5), None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, idx), Some(expected));
}

#[test]
fn per_item_details_without_usage_is_none() {
    let mut result = response(2, Some(10), Some(details(&[("cached_tokens", integer(4))])));
    result.usage = None;
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_without_details_is_none() {
    let result = response(2, Some(10), None);
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_empty_map_is_none() {
    let result = response(2, Some(10), Some(Extra::new()));
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_all_null_is_none() {
    let result = response(1, Some(10), Some(details(&[("cached_tokens", Json::Null)])));
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_null_fields_are_dropped() {
    let result = response(
        1,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(4)),
            ("image_tokens", Json::Null),
        ])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("cached_tokens", integer(4))]))
    );
}

#[test]
fn per_item_details_single_item_keeps_every_value() {
    let prompt_details = details(&[
        ("cached_tokens", integer(4)),
        ("ratio", decimal(1.5)),
        ("note", Json::Text("x".to_owned())),
    ]);
    let result = response(1, Some(10), Some(prompt_details.clone()));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(prompt_details)
    );
}

#[rstest]
#[case::first_index(0, 3, 1)]
#[case::second_index(1, 2, 1)]
#[case::last_index(2, 2, 0)]
fn per_item_details_distributes_integers(
    #[case] idx: usize,
    #[case] expected_cached: i64,
    #[case] expected_image: i64,
) {
    let result = response(
        3,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(7)),
            ("image_tokens", integer(2)),
        ])),
    );
    let distributed =
        tokens::per_item_prompt_tokens_details(&result, idx).expect("the details distribute");
    assert_eq!(
        distributed.get("cached_tokens"),
        Some(&integer(expected_cached))
    );
    assert_eq!(
        distributed.get("image_tokens"),
        Some(&integer(expected_image))
    );
}

#[test]
fn per_item_details_divides_floats() {
    let result = response(2, Some(10), Some(details(&[("ratio", decimal(1.5))])));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("ratio", decimal(0.75))]))
    );
}

#[test]
fn per_item_details_passes_other_values_through() {
    let prompt_details = details(&[
        ("note", Json::Text("x".to_owned())),
        ("flag", Json::Bool(true)),
        ("nothing", Json::Null),
    ]);
    let result = response(2, Some(10), Some(prompt_details));
    let expected = details(&[
        ("note", Json::Text("x".to_owned())),
        ("flag", Json::Bool(true)),
    ]);
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(expected.clone())
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(expected)
    );
}

#[test]
fn per_item_details_negative_integer_floors() {
    let result = response(
        2,
        Some(10),
        Some(details(&[("cached_tokens", integer(-5))])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("cached_tokens", integer(-2))]))
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("cached_tokens", integer(-3))]))
    );
}

#[test]
fn per_item_details_empty_data_keeps_every_value() {
    let prompt_details = details(&[("cached_tokens", integer(7))]);
    let result = response(0, Some(10), Some(prompt_details.clone()));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 4),
        Some(prompt_details)
    );
}

#[test]
fn add_embedding_entry_derives_key_and_envelope() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("model", text("text-embedding-3-small"))]);
    let result = response(1, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    assert!(!entry.cache_key.is_empty());
    assert_eq!(
        cache_request.cache_key.as_deref(),
        Some(entry.cache_key.as_str())
    );
    assert_eq!(entry.ttl, Some(600.0_f64));
    assert_envelope(&entry);
    let stored = cached(&entry);
    assert_eq!(stored.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(stored.prompt_tokens, Some(10));
    assert_eq!(stored.format_version, EMBEDDING_CACHE_FORMAT_VERSION);
}

#[test]
fn add_embedding_entry_derives_the_key_from_the_item_input() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![
        param("model", text("m")),
        param("input", inputs(&["hello", "world"])),
    ]);
    let result = response(2, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    let mut reference = request(vec![
        param("model", text("m")),
        param("input", text("hello")),
    ]);
    let expected = get_cache_key(&mut reference.key, &cache.key_settings, &cache.hooks)
        .expect("the reference key derives");
    assert_eq!(entry.cache_key, expected);
}

#[test]
fn add_embedding_entry_restores_the_request_input() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let original = inputs(&["hello", "world"]);
    let mut cache_request = request(vec![
        param("model", text("text-embedding-3-small")),
        param("input", original.clone()),
    ]);
    let result = response(1, Some(6), None);
    add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
        .expect("the entry is built");
    let stored_input = cache_request
        .key
        .params
        .iter()
        .find(|param| param.name == "input")
        .map(|param| param.value.clone());
    assert_eq!(stored_input, Some(original));
}

#[test]
fn add_embedding_entry_appends_the_input_for_the_key_only() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("model", text("m"))]);
    let result = response(1, Some(6), None);
    add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
        .expect("the entry is built");
    assert!(
        cache_request
            .key
            .params
            .iter()
            .all(|param| param.name != "input")
    );
}

#[test]
fn add_embedding_entry_uses_item_tokens_and_details() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(
        2,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(3)),
            ("image_tokens", integer(2)),
        ])),
    );
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    let stored = cached(&entry);
    assert_eq!(stored.prompt_tokens, Some(5));
    assert_eq!(
        stored.prompt_tokens_details,
        Some(details(&[
            ("cached_tokens", integer(2)),
            ("image_tokens", integer(1)),
        ]))
    );
}

#[test]
fn add_embedding_entry_missing_item_errors() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    let error =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 1)
            .expect_err("the response has no second item");
    assert!(matches!(
        error,
        EmbeddingCacheError::MissingResponseItem { index: 1 }
    ));
}

#[test]
fn add_embedding_entry_control_ttl_wins() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.control.ttl = Some(30);
    let result = response(1, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    assert_eq!(entry.ttl, Some(30.0_f64));
}

#[tokio::test]
async fn pipeline_skips_when_cache_is_disabled() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOff,
        settings(None),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert!(cache_request.cache_key.is_none());
}

#[tokio::test]
async fn pipeline_writes_when_opted_in() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOff,
        settings(None),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.key.cache_control = Some(RequestCacheControl {
        namespace: None,
        use_cache: true,
    });
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(recording.sets().len(), 1);
}

#[tokio::test]
async fn pipeline_skips_more_inputs_than_embeddings() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("DEBUG", "skipping embedding cache write"),
        1
    );
}

#[tokio::test]
async fn pipeline_skips_more_embeddings_than_inputs() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("DEBUG", "skipping embedding cache write"),
        1
    );
}

#[tokio::test]
async fn pipeline_writes_every_entry_with_the_settings_ttl() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    let writes = recording.sets();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes.first().map(|write| write.2), Some(Some(600.0_f64)));
    assert_eq!(writes.get(1).map(|write| write.2), Some(Some(600.0_f64)));
    assert_ne!(
        writes.first().map(|write| write.0.clone()),
        writes.get(1).map(|write| write.0.clone())
    );
    assert_eq!(
        cache_request.cache_key.as_deref(),
        writes.last().map(|write| write.0.as_str())
    );
}

#[tokio::test]
async fn pipeline_control_ttl_overrides_settings_ttl() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.control.ttl = Some(30);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(
        recording.sets().first().map(|write| write.2),
        Some(Some(30.0_f64))
    );
}

#[tokio::test]
async fn pipeline_uses_the_dynamic_store_when_injected() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore::default();
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    let dynamic_ref: Option<&dyn CacheBackend> = Some(&dynamic);
    async_add_cache_pipeline(&cache, dynamic_ref, None, &mut cache_request, &result).await;
    let writes = dynamic.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes.first().map(|write| write.0.len()), Some(2));
    assert_eq!(writes.first().map(|write| write.1), Some(Some(600.0_f64)));
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_uses_native_batch_storage_when_configured() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore::default();
    let native = RecordingNative::default();
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(
        &cache,
        Some(&dynamic),
        Some(&native),
        &mut cache_request,
        &result,
    )
    .await;
    let requests = native.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(native.responses().len(), 2);
    assert!(dynamic.writes().is_empty());
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_native_filters_missing_requests() {
    let recording = Arc::new(RecordingCache::default());
    let native = RecordingNative {
        failure: false,
        skip_last: true,
        calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, Some(&native), &mut cache_request, &result).await;
    assert_eq!(native.requests().len(), 1);
    assert_eq!(native.responses().len(), 1);
}

#[tokio::test]
async fn pipeline_native_failure_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let native = RecordingNative {
        failure: true,
        skip_last: false,
        calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, Some(&native), &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#[tokio::test]
async fn pipeline_store_failure_is_logged() {
    let recording = Arc::new(RecordingCache {
        failure: true,
        sets: Mutex::new(Vec::new()),
    });
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#[tokio::test]
async fn pipeline_dynamic_failure_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore {
        failure: true,
        writes: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    let dynamic_ref: Option<&dyn CacheBackend> = Some(&dynamic);
    async_add_cache_pipeline(&cache, dynamic_ref, None, &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_missing_input_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("model", text("m"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#[test]
fn per_item_details_splits_by_declared_field_type() {
    let result = response(
        3,
        Some(10),
        Some(details(&[
            ("video_length_seconds", integer(10)),
            ("cache_creation_tokens", integer(5)),
        ])),
    );
    let first = tokens::per_item_prompt_tokens_details(&result, 0).expect("the details split");
    assert_eq!(
        first.get("video_length_seconds"),
        Some(&decimal(3.333_333_333_333_333_5_f64))
    );
    assert_eq!(first.get("cache_write_tokens"), Some(&integer(2)));
    assert_eq!(first.get("cache_creation_tokens"), Some(&integer(2)));
    let last = tokens::per_item_prompt_tokens_details(&result, 2).expect("the details split");
    assert_eq!(last.get("cache_write_tokens"), Some(&integer(1)));
    assert_eq!(
        last.get("video_length_seconds"),
        Some(&decimal(3.333_333_333_333_333_5_f64))
    );
}

#[test]
fn per_item_details_coerces_an_integral_float_for_an_int_field() {
    let result = response(
        2,
        Some(10),
        Some(details(&[("cached_tokens", decimal(4.0))])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("cached_tokens", integer(2))]))
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("cached_tokens", integer(2))]))
    );
}

#[test]
fn per_item_details_mirrors_the_cache_token_keys() {
    let write_only = response(
        1,
        Some(10),
        Some(details(&[("cache_write_tokens", integer(3))])),
    );
    let write_mirrored =
        tokens::per_item_prompt_tokens_details(&write_only, 0).expect("the details");
    assert_eq!(write_mirrored.get("cache_write_tokens"), Some(&integer(3)));
    assert_eq!(
        write_mirrored.get("cache_creation_tokens"),
        Some(&integer(3))
    );
    let creation_only = response(
        1,
        Some(10),
        Some(details(&[("cache_creation_tokens", integer(5))])),
    );
    let creation_mirrored =
        tokens::per_item_prompt_tokens_details(&creation_only, 0).expect("the details");
    assert_eq!(
        creation_mirrored.get("cache_write_tokens"),
        Some(&integer(5))
    );
    assert_eq!(
        creation_mirrored.get("cache_creation_tokens"),
        Some(&integer(5))
    );
}

#[test]
fn per_item_details_write_tokens_win_the_mirror() {
    let both = response(
        1,
        Some(10),
        Some(details(&[
            ("cache_write_tokens", integer(3)),
            ("cache_creation_tokens", integer(5)),
        ])),
    );
    let both_mirrored = tokens::per_item_prompt_tokens_details(&both, 0).expect("the details");
    assert_eq!(both_mirrored.get("cache_write_tokens"), Some(&integer(3)));
    assert_eq!(
        both_mirrored.get("cache_creation_tokens"),
        Some(&integer(3))
    );
}

#[test]
fn per_item_details_mirrors_cache_creation_input_tokens() {
    let nested = response(
        1,
        Some(10),
        Some(details(&[("cache_creation_input_tokens", integer(9))])),
    );
    let nested_mirrored = tokens::per_item_prompt_tokens_details(&nested, 0).expect("the details");
    assert_eq!(nested_mirrored.get("cache_write_tokens"), Some(&integer(9)));
    assert_eq!(
        nested_mirrored.get("cache_creation_tokens"),
        Some(&integer(9))
    );
    assert_eq!(
        nested_mirrored.get("cache_creation_input_tokens"),
        Some(&integer(9))
    );
}

#[test]
fn per_item_details_audio_length_uses_true_division() {
    let result = response(
        2,
        Some(10),
        Some(details(&[("audio_length_seconds", integer(5))])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("audio_length_seconds", decimal(2.5))]))
    );
}

#[test]
fn per_item_details_extra_integer_value_divides() {
    let result = response(2, Some(10), Some(details(&[("ratio_count", integer(5))])));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("ratio_count", integer(3))]))
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("ratio_count", integer(2))]))
    );
}

async fn read_embedding(cache: &TestCache, input: &str) -> CachedEmbedding {
    let mut read_request = request(vec![param("model", text("m")), param("input", text(input))]);
    let cached = cache
        .async_get_cache(None, &mut read_request)
        .await
        .expect("the input is cached");
    CacheCodec::decode(Some(&cached)).expect("the cached embedding decodes")
}

#[tokio::test]
async fn pipeline_keys_each_batch_input_on_its_own_input() {
    let storing = Arc::new(StoringBackend::default());
    let cache = cache(Arc::clone(&storing), CacheMode::DefaultOn, settings(None));
    let mut write_request = request(vec![
        param("model", text("m")),
        param("input", inputs(&["a", "b"])),
    ]);
    write_request.key.litellm_params = Some(LiteLlmParams {
        preset_cache_key: None,
        metadata: None,
        litellm_metadata: None,
        file_name: None,
    });
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut write_request, &result).await;
    let first = read_embedding(&cache, "a").await;
    assert_eq!(
        first.embedding,
        Some(EmbeddingPayload::Vector(vec![Number::from(0_u64)]))
    );
    let second = read_embedding(&cache, "b").await;
    assert_eq!(
        second.embedding,
        Some(EmbeddingPayload::Vector(vec![Number::from(1_u64)]))
    );
}

#[tokio::test]
async fn pipeline_ignores_a_caller_preset_for_each_item() {
    let storing = Arc::new(StoringBackend::default());
    let cache = cache(Arc::clone(&storing), CacheMode::DefaultOn, settings(None));
    let mut write_request = request(vec![
        param("model", text("m")),
        param("input", inputs(&["a", "b"])),
    ]);
    write_request.key.litellm_params = Some(LiteLlmParams {
        preset_cache_key: Some("preset".to_owned()),
        metadata: None,
        litellm_metadata: None,
        file_name: None,
    });
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut write_request, &result).await;
    let first = read_embedding(&cache, "a").await;
    assert_eq!(
        first.embedding,
        Some(EmbeddingPayload::Vector(vec![Number::from(0_u64)]))
    );
    let second = read_embedding(&cache, "b").await;
    assert_eq!(
        second.embedding,
        Some(EmbeddingPayload::Vector(vec![Number::from(1_u64)]))
    );
    let preset = write_request
        .key
        .litellm_params
        .as_ref()
        .and_then(|params| params.preset_cache_key.as_deref());
    assert_eq!(preset, Some("preset"));
}
