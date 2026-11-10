#![cfg(test)]

use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;

use super::support_tests::{
    BatchStore, Failure, FakeBackend, NOW, cache, control, envelope, fake_cache, key_settings,
    messages, number, object, reply, reply_response, with_params,
};
use crate::backend::CacheBackend;
use crate::backend::memory::MemoryCache;
use crate::backend::memory::local::LocalMemoryOps;
use crate::cache_key::request::{KeyParam, LiteLlmParams};
use crate::cache_key::value::KeyValue;
use crate::envelope::backend::{EnvelopeBackend, StoredBackend};
use crate::envelope::facade::{EnvelopeSettings, WriteError};
use crate::envelope::lookup::SEMANTIC_SIMILARITY;
use crate::envelope::request::EnvelopeRequest;
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};

fn write_request() -> EnvelopeRequest {
    let mut request = with_params(vec![KeyParam {
        name: "model".to_owned(),
        value: KeyValue::String("gpt-4".to_owned()),
    }]);
    request.cache_key = Some("key-1".to_owned());
    request
}

#[test]
fn get_cache_returns_none_when_the_cache_is_off() {
    let fake = Arc::new(FakeBackend::new(Some(reply())));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOff),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), None);
    assert!(fake.gets().is_empty());
}

#[test]
fn get_cache_returns_none_when_no_cache_key_is_derived() {
    let fake = Arc::new(FakeBackend::new(Some(reply())));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "model".to_owned(),
        value: KeyValue::String("gpt-4".to_owned()),
    }]);
    request.key.metadata = Some(object(&[(
        "caching_groups",
        KeyValue::String("bad".to_owned()),
    )]));
    assert_eq!(cache.get_cache(None, &mut request), None);
    assert!(fake.gets().is_empty());
    assert!(
        messages(&cache.hooks)
            .iter()
            .any(|message| message.starts_with("An exception occurred:"))
    );
}

#[test]
fn get_cache_uses_a_preset_cache_key() {
    let fake = Arc::new(FakeBackend::new(Some(reply())));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), Some(reply_response()));
    let gets = fake.gets();
    assert_eq!(gets.len(), 1);
    assert_eq!(gets[0].0, "key-1");
}

#[test]
fn get_cache_stores_the_derived_key_for_reuse() {
    let fake = Arc::new(FakeBackend::new(Some(reply())));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "model".to_owned(),
        value: KeyValue::String("gpt-4".to_owned()),
    }]);
    request.key.litellm_params = Some(LiteLlmParams {
        preset_cache_key: None,
        metadata: None,
        litellm_metadata: None,
        file_name: None,
    });
    assert_eq!(cache.get_cache(None, &mut request), Some(reply_response()));
    let gets = fake.gets();
    let derived = gets[0].0.clone();
    assert_eq!(
        request
            .key
            .litellm_params
            .as_ref()
            .and_then(|params| params.preset_cache_key.clone()),
        Some(derived)
    );
}

#[test]
fn get_cache_returns_none_for_a_stale_response() {
    let stale = envelope(Some(NOW - 120.0_f64), reply_response());
    let fake = Arc::new(FakeBackend::new(Some(stale)));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    request.control = control(None, Some(60), None);
    assert_eq!(cache.get_cache(None, &mut request), None);
}

#[test]
fn get_cache_passes_only_the_safe_lookup() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![
        KeyParam {
            name: "model".to_owned(),
            value: KeyValue::String("gpt-4".to_owned()),
        },
        KeyParam {
            name: "messages".to_owned(),
            value: KeyValue::String("hi".to_owned()),
        },
        KeyParam {
            name: "temperature".to_owned(),
            value: number(0.7_f64),
        },
    ]);
    request.cache_key = Some("key-1".to_owned());
    request.key.metadata = Some(object(&[(
        "user_api_key",
        KeyValue::String("key-hash".to_owned()),
    )]));
    assert_eq!(cache.get_cache(None, &mut request), None);
    let gets = fake.gets();
    let (_, lookup) = &gets[0];
    assert_eq!(lookup.messages, Some(KeyValue::String("hi".to_owned())));
    assert_eq!(lookup.input, None);
    assert_eq!(
        lookup.metadata,
        Some(object(&[(
            "user_api_key",
            KeyValue::String("key-hash".to_owned())
        )]))
    );
}

#[test]
fn get_cache_updates_metadata_from_the_lookup() {
    let mut configured = FakeBackend::new(None);
    configured.stamp = Some(0.42_f64);
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "messages".to_owned(),
        value: KeyValue::String("hi".to_owned()),
    }]);
    request.cache_key = Some("key-1".to_owned());
    request.key.metadata = Some(object(&[(
        "user_api_key",
        KeyValue::String("key-hash".to_owned()),
    )]));
    assert_eq!(cache.get_cache(None, &mut request), None);
    assert_eq!(
        request.key.metadata,
        Some(object(&[
            ("user_api_key", KeyValue::String("key-hash".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.42_f64)),
        ]))
    );
    let gets = fake.gets();
    let (_, lookup) = &gets[0];
    assert_eq!(
        lookup.metadata,
        Some(object(&[
            ("user_api_key", KeyValue::String("key-hash".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.42_f64)),
        ]))
    );
}

#[test]
fn get_cache_prefers_the_dynamic_backend() {
    let dynamic = Arc::new(FakeBackend::new(Some(reply())));
    let fallback = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fallback,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let dynamic_ref: &dyn EnvelopeBackend = dynamic.as_ref();
    let mut request = write_request();
    assert_eq!(
        cache.get_cache(Some(dynamic_ref), &mut request),
        Some(reply_response())
    );
    assert!(fallback.gets().is_empty());
    assert_eq!(dynamic.gets().len(), 1);
}

#[test]
fn get_cache_logs_a_backend_failure_and_returns_none() {
    let mut configured = FakeBackend::new(None);
    configured.failure = Failure::Get;
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), None);
    assert!(
        messages(&cache.hooks).iter().any(|message| {
            message.starts_with("An exception occurred: the get_cache operation")
        })
    );
}

#[test]
fn get_cache_logs_a_decode_failure_and_returns_none() {
    let broken = envelope(Some(NOW), CacheValue::Text("not a literal".to_owned()));
    let fake = Arc::new(FakeBackend::new(Some(broken)));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), None);
    assert!(messages(&cache.hooks).iter().any(|message| {
        message.starts_with("An exception occurred: the cached value is neither JSON")
    }));
}

#[test]
fn get_cache_reads_through_the_memory_backend() {
    let operations = LocalMemoryOps::new(1024, Arc::new(|| NOW)).expect("a memory cache");
    let memory: MemoryCache<LocalMemoryOps> = MemoryCache { operations };
    let stored = StoredBackend {
        backend: Arc::new(memory),
    };
    let cache = cache(
        Arc::new(stored),
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache.add_cache(&mut write_request(), &reply_response());
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), Some(reply_response()));
}

#[tokio::test]
async fn async_get_cache_reads_through_the_memory_backend() {
    let operations = LocalMemoryOps::new(1024, Arc::new(|| NOW)).expect("a memory cache");
    let memory: MemoryCache<LocalMemoryOps> = MemoryCache { operations };
    let stored = StoredBackend {
        backend: Arc::new(memory),
    };
    let cache = cache(
        Arc::new(stored),
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache.add_cache(&mut write_request(), &reply_response());
    let mut request = write_request();
    assert_eq!(
        cache.async_get_cache(None, &mut request).await,
        Some(reply_response())
    );
}

#[tokio::test]
async fn async_add_cache_writes_through_the_memory_backend() {
    let operations = LocalMemoryOps::new(1024, Arc::new(|| NOW)).expect("a memory cache");
    let memory: MemoryCache<LocalMemoryOps> = MemoryCache { operations };
    let stored = StoredBackend {
        backend: Arc::new(memory),
    };
    let cache = cache(
        Arc::new(stored),
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    let mut request = write_request();
    assert_eq!(cache.get_cache(None, &mut request), Some(reply_response()));
}

#[tokio::test]
async fn batch_cache_write_delegates_to_the_stored_backend() {
    let store = Arc::new(BatchStore::default());
    let plain: Arc<BatchStore> = Arc::clone(&store);
    let backend: Arc<dyn CacheBackend> = plain;
    let cache = cache(
        Arc::new(StoredBackend { backend }),
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache
        .batch_cache_write(&mut write_request(), &reply_response())
        .await
        .expect("a cache write");
    let batches = store.batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].1, envelope(Some(NOW), reply_response()));
    assert_eq!(batches[0].2, None);
}

#[tokio::test]
async fn async_get_cache_returns_none_when_the_cache_is_off() {
    let fake = Arc::new(FakeBackend::new(Some(reply())));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOff),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.async_get_cache(None, &mut request).await, None);
    assert!(fake.gets().is_empty());
}

#[tokio::test]
async fn async_get_cache_prefers_the_dynamic_backend() {
    let dynamic = Arc::new(FakeBackend::new(Some(reply())));
    let fallback = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fallback,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let dynamic_ref: &dyn EnvelopeBackend = dynamic.as_ref();
    let mut request = write_request();
    assert_eq!(
        cache.async_get_cache(Some(dynamic_ref), &mut request).await,
        Some(reply_response())
    );
    assert!(fallback.gets().is_empty());
}

#[tokio::test]
async fn async_get_cache_logs_a_backend_failure() {
    let mut configured = FakeBackend::new(None);
    configured.failure = Failure::Get;
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = write_request();
    assert_eq!(cache.async_get_cache(None, &mut request).await, None);
    assert!(
        messages(&cache.hooks)
            .iter()
            .any(|message| message.starts_with("An exception occurred:"))
    );
}

#[tokio::test]
async fn async_get_cache_updates_metadata_from_the_lookup() {
    let mut configured = FakeBackend::new(None);
    configured.stamp = Some(0.42_f64);
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "messages".to_owned(),
        value: KeyValue::String("hi".to_owned()),
    }]);
    request.cache_key = Some("key-1".to_owned());
    request.key.metadata = Some(object(&[(
        "user_api_key",
        KeyValue::String("key-hash".to_owned()),
    )]));
    assert_eq!(cache.async_get_cache(None, &mut request).await, None);
    assert_eq!(
        request.key.metadata,
        Some(object(&[
            ("user_api_key", KeyValue::String("key-hash".to_owned())),
            (SEMANTIC_SIMILARITY, number(0.42_f64)),
        ]))
    );
}

#[test]
fn add_cache_wraps_the_result_in_a_timestamp_envelope() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache.add_cache(&mut write_request(), &reply_response());
    let sets = fake.sets();
    assert_eq!(sets.len(), 1);
    let (key, value, ttl) = &sets[0];
    assert_eq!(key, "key-1");
    assert_eq!(value, &envelope(Some(NOW), reply_response()));
    assert_eq!(ttl, &None);
}

#[test]
fn add_cache_applies_the_control_ttl_over_the_default() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings {
            ttl: Some(600.0_f64),
            redis_flush_size: None,
        },
    );
    let mut request = write_request();
    request.control = control(Some(30), None, None);
    cache.add_cache(&mut request, &reply_response());
    assert_eq!(fake.sets()[0].2, Some(30.0_f64));
}

#[test]
fn add_cache_applies_the_default_ttl_without_a_control() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings {
            ttl: Some(600.0_f64),
            redis_flush_size: None,
        },
    );
    cache.add_cache(&mut write_request(), &reply_response());
    assert_eq!(fake.sets()[0].2, Some(600.0_f64));
}

#[test]
fn add_cache_skips_when_the_cache_is_off() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOff),
        EnvelopeSettings::default(),
    );
    cache.add_cache(&mut write_request(), &reply_response());
    assert!(fake.sets().is_empty());
}

#[test]
fn add_cache_logs_a_backend_failure() {
    let mut configured = FakeBackend::new(None);
    configured.failure = Failure::Set;
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache.add_cache(&mut write_request(), &reply_response());
    assert!(
        messages(&cache.hooks)
            .iter()
            .any(|message| message.starts_with("LiteLLM Cache: exception in add_cache:"))
    );
}

#[test]
fn add_cache_logs_a_key_failure() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "model".to_owned(),
        value: KeyValue::String("gpt-4".to_owned()),
    }]);
    request.key.metadata = Some(object(&[(
        "caching_groups",
        KeyValue::String("bad".to_owned()),
    )]));
    cache.add_cache(&mut request, &reply_response());
    assert!(fake.sets().is_empty());
    assert!(
        messages(&cache.hooks)
            .iter()
            .any(|message| message.starts_with("LiteLLM Cache: exception in add_cache:"))
    );
}

#[tokio::test]
async fn async_add_cache_writes_through_the_async_backend() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    let async_sets = fake.async_sets();
    assert_eq!(async_sets.len(), 1);
    assert!(fake.sets().is_empty());
    assert_eq!(async_sets[0].1, envelope(Some(NOW), reply_response()));
}

#[tokio::test]
async fn async_add_cache_batches_when_redis_flushes() {
    let fake = Arc::new(FakeBackend::new(None));
    let mut settings = key_settings(CacheMode::DefaultOn);
    settings.cache_type = LiteLlmCacheType::Redis;
    let cache = fake_cache(
        &fake,
        settings,
        EnvelopeSettings {
            ttl: None,
            redis_flush_size: Some(10),
        },
    );
    let mut request = write_request();
    request.control = control(Some(30), None, None);
    cache
        .async_add_cache(None, &mut request, &reply_response())
        .await;
    let batches = fake.batches();
    assert_eq!(batches.len(), 1);
    assert!(fake.async_sets().is_empty());
    assert_eq!(batches[0].2, Some(30.0_f64));
}

#[tokio::test]
async fn async_add_cache_writes_directly_for_redis_without_a_flush_size() {
    let fake = Arc::new(FakeBackend::new(None));
    let mut settings = key_settings(CacheMode::DefaultOn);
    settings.cache_type = LiteLlmCacheType::Redis;
    let cache = fake_cache(&fake, settings, EnvelopeSettings::default());
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    assert_eq!(fake.async_sets().len(), 1);
    assert!(fake.batches().is_empty());
}

#[tokio::test]
async fn async_add_cache_writes_directly_for_local_with_a_flush_size() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings {
            ttl: None,
            redis_flush_size: Some(10),
        },
    );
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    assert_eq!(fake.async_sets().len(), 1);
    assert!(fake.batches().is_empty());
}

#[tokio::test]
async fn async_add_cache_uses_the_dynamic_backend() {
    let dynamic = Arc::new(FakeBackend::new(None));
    let fallback = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fallback,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let dynamic_ref: &dyn EnvelopeBackend = dynamic.as_ref();
    cache
        .async_add_cache(Some(dynamic_ref), &mut write_request(), &reply_response())
        .await;
    assert_eq!(dynamic.async_sets().len(), 1);
    assert!(fallback.async_sets().is_empty());
}

#[tokio::test]
async fn async_add_cache_logs_a_batch_failure() {
    let mut configured = FakeBackend::new(None);
    configured.failure = Failure::Batch;
    let fake = Arc::new(configured);
    let mut settings = key_settings(CacheMode::DefaultOn);
    settings.cache_type = LiteLlmCacheType::Redis;
    let cache = fake_cache(
        &fake,
        settings,
        EnvelopeSettings {
            ttl: None,
            redis_flush_size: Some(10),
        },
    );
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    assert!(
        messages(&cache.hooks)
            .iter()
            .any(|message| message.starts_with("LiteLLM Cache: exception in add_cache:"))
    );
}

#[tokio::test]
async fn async_add_cache_skips_when_the_cache_is_off() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOff),
        EnvelopeSettings::default(),
    );
    cache
        .async_add_cache(None, &mut write_request(), &reply_response())
        .await;
    assert!(fake.async_sets().is_empty());
    assert!(fake.batches().is_empty());
}

#[tokio::test]
async fn batch_cache_write_wraps_and_delegates() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings {
            ttl: Some(600.0_f64),
            redis_flush_size: None,
        },
    );
    let mut request = write_request();
    request.control = control(Some(30), None, None);
    cache
        .batch_cache_write(&mut request, &reply_response())
        .await
        .expect("a cache write");
    let batches = fake.batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].0, "key-1");
    assert_eq!(batches[0].1, envelope(Some(NOW), reply_response()));
    assert_eq!(batches[0].2, Some(30.0_f64));
}

#[tokio::test]
async fn batch_cache_write_reports_a_key_failure() {
    let fake = Arc::new(FakeBackend::new(None));
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    let mut request = with_params(vec![KeyParam {
        name: "model".to_owned(),
        value: KeyValue::String("gpt-4".to_owned()),
    }]);
    request.key.metadata = Some(object(&[(
        "caching_groups",
        KeyValue::String("bad".to_owned()),
    )]));
    assert!(matches!(
        cache
            .batch_cache_write(&mut request, &reply_response())
            .await,
        Err(WriteError::Key { .. })
    ));
    assert!(fake.batches().is_empty());
}

#[tokio::test]
async fn batch_cache_write_reports_a_backend_failure() {
    let mut configured = FakeBackend::new(None);
    configured.failure = Failure::Batch;
    let fake = Arc::new(configured);
    let cache = fake_cache(
        &fake,
        key_settings(CacheMode::DefaultOn),
        EnvelopeSettings::default(),
    );
    assert!(matches!(
        cache
            .batch_cache_write(&mut write_request(), &reply_response())
            .await,
        Err(WriteError::Backend { .. })
    ));
}
