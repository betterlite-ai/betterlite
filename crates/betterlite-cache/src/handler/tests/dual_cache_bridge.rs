use betterlite_context::local_ttl_cache::CacheValue;

use super::support::{cache_object, cache_text, dual_cache, text};
use crate::backend::BackendError;
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::lookup::CacheLookup;
use crate::handler::dynamic_cache::DualCacheEnvelope;

fn stored() -> CacheValue {
    cache_object(&[("answer", cache_text("42"))])
}

#[tokio::test]
async fn async_reads_go_through_the_memory_tier() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    EnvelopeBackend::set_cache(&envelope, "key", stored(), None)
        .expect("the value is representable");
    let mut lookup = CacheLookup::default();
    let read = EnvelopeBackend::async_get_cache(&envelope, "key", &mut lookup)
        .await
        .expect("the read succeeds");
    assert_eq!(read, Some(stored()));
}

#[test]
fn sync_reads_go_through_the_memory_tier() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    EnvelopeBackend::set_cache(&envelope, "key", stored(), None)
        .expect("the value is representable");
    let mut lookup = CacheLookup::default();
    let read =
        EnvelopeBackend::get_cache(&envelope, "key", &mut lookup).expect("the read succeeds");
    assert_eq!(read, Some(stored()));
}

#[tokio::test]
async fn missing_keys_read_as_none() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    let mut lookup = CacheLookup::default();
    let read = EnvelopeBackend::async_get_cache(&envelope, "missing", &mut lookup)
        .await
        .expect("the read succeeds");
    assert_eq!(read, None);
}

#[tokio::test]
async fn async_set_stores_the_value() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    EnvelopeBackend::async_set_cache(&envelope, "key", stored(), None)
        .await
        .expect("the write succeeds");
    let mut lookup = CacheLookup::default();
    let read =
        EnvelopeBackend::get_cache(&envelope, "key", &mut lookup).expect("the read succeeds");
    assert_eq!(read, Some(stored()));
}

#[tokio::test]
async fn batch_write_stores_the_value() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    EnvelopeBackend::batch_cache_write(&envelope, "key", stored(), None)
        .await
        .expect("the write succeeds");
    let mut lookup = CacheLookup::default();
    let read =
        EnvelopeBackend::get_cache(&envelope, "key", &mut lookup).expect("the read succeeds");
    assert_eq!(read, Some(stored()));
}

#[tokio::test]
async fn values_round_trip_through_the_dual_cache() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    let value = cache_object(&[
        ("array", CacheValue::Array(vec![CacheValue::Integer(1)])),
        ("boolean", CacheValue::Boolean(true)),
        ("float", CacheValue::Float(0.5)),
        ("nested", cache_object(&[("text", cache_text("nested"))])),
        ("null", CacheValue::Null),
        ("number", CacheValue::Integer(-3)),
    ]);
    EnvelopeBackend::set_cache(&envelope, "key", value.clone(), None)
        .expect("the value is representable");
    let read = EnvelopeBackend::async_get_cache(&envelope, "key", &mut CacheLookup::default())
        .await
        .expect("the read succeeds");
    assert_eq!(read, Some(value));
}

#[test]
fn an_unrepresentable_value_is_rejected() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    let result = EnvelopeBackend::set_cache(&envelope, "key", CacheValue::Bytes(vec![1, 2]), None);
    assert!(matches!(result, Err(BackendError::Unsupported(_))));
}

#[tokio::test]
async fn the_lookup_projection_is_ignored() {
    let dual = dual_cache();
    let envelope = DualCacheEnvelope { cache: &dual };
    EnvelopeBackend::set_cache(&envelope, "key", stored(), None)
        .expect("the value is representable");
    let mut lookup = CacheLookup {
        messages: Some(text("hello")),
        input: Some(text("hello")),
        metadata: None,
    };
    let read = EnvelopeBackend::async_get_cache(&envelope, "key", &mut lookup)
        .await
        .expect("the read succeeds");
    assert_eq!(read, Some(stored()));
}
