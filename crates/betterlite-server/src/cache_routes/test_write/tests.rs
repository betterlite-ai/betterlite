#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::sync::Arc;

use tracing::subscriber::{DefaultGuard, set_default};

use betterlite_cache::backend::CacheBackend as _;
use betterlite_cache::cache_key::value::KeyValue;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink as _, LoggerSink as _};
use betterlite_redact::structured::Payload;

use super::super::redis_support::{breaker_open_redis_cache, local_cache, unreachable_redis_cache};
use super::{ErrorLogger, RedisValues, SilentConsole, add_test_key, test_messages};

fn local_bridge() -> RedisValues {
    RedisValues {
        cache: Arc::new(local_cache()),
    }
}

fn capture_logs() -> (tempfile::NamedTempFile, DefaultGuard) {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

#[tokio::test]
async fn the_bridge_refuses_a_cache_without_a_redis_backend() {
    let bridge = local_bridge();
    let error = bridge
        .async_set_cache("key", CacheValue::Text("value".to_owned()), None)
        .await
        .expect_err("a local cache has no redis values");
    assert_eq!(
        error.to_string(),
        "the async_set_cache operation is not implemented by this cache backend"
    );
    let batch_error = bridge
        .batch_cache_write("key", CacheValue::Text("value".to_owned()), None)
        .await
        .expect_err("a local cache has no redis values");
    assert_eq!(
        batch_error.to_string(),
        "the batch_cache_write operation is not implemented by this cache backend"
    );
}

#[tokio::test]
async fn the_bridge_swallows_a_redis_write_failure() {
    let (log, _guard) = capture_logs();
    let bridge = RedisValues {
        cache: Arc::new(breaker_open_redis_cache().await),
    };
    bridge
        .async_set_cache("key", CacheValue::Text("value".to_owned()), None)
        .await
        .expect("a failed write is logged, not raised");
    bridge
        .batch_cache_write("key", CacheValue::Text("value".to_owned()), None)
        .await
        .expect("a failed write is logged, not raised");
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.contains(
        "LiteLLM Cache: exception in add_cache: Redis circuit breaker is open, skipping async_set_cache"
    ));
    assert!(written.contains(
        "LiteLLM Cache: exception in add_cache: Redis circuit breaker is open, skipping batch_cache_write"
    ));
}

#[test]
fn the_bridge_reports_the_redis_default_ttl() {
    assert_eq!(local_bridge().default_ttl(), 60_i64);
}

#[test]
fn the_logger_only_reports_errors() {
    assert!(!ErrorLogger.debug(&Payload::Null));
    assert!(!ErrorLogger.info(&Payload::Null));
    assert!(!ErrorLogger.debug_enabled());
    assert!(ErrorLogger.error(&Payload::String("boom".to_owned())));
}

#[test]
fn the_console_prints_nothing() {
    assert!(!SilentConsole.print(&Payload::Null));
}

#[test]
fn the_logger_writes_the_error_payload() {
    let (log, _guard) = capture_logs();
    assert!(ErrorLogger.error(&Payload::String("boom".to_owned())));
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.contains("boom"));
}

#[test]
fn the_test_message_has_litellms_shape() {
    let KeyValue::Array(items) = test_messages() else {
        panic!("the test messages are an array");
    };
    assert_eq!(items.len(), 1);
    let KeyValue::Object(message) = &items[0] else {
        panic!("a message is an object");
    };
    assert_eq!(
        message.get("role"),
        Some(&KeyValue::String("user".to_owned()))
    );
    assert_eq!(
        message.get("content"),
        Some(&KeyValue::String("test from litellm".to_owned()))
    );
}

#[test]
fn the_envelope_uses_the_injected_clock() {
    let envelope = super::test_envelope(&Arc::new(local_cache()), Arc::new(|| 1_234.5_f64))
        .expect("the envelope opens");
    assert_eq!((envelope.clock)().to_bits(), 1_234.5_f64.to_bits());
}

#[tokio::test]
async fn add_test_key_accepts_a_redis_cache() {
    add_test_key(
        Arc::new(unreachable_redis_cache().await),
        Arc::new(|| 1_700_000_000.0_f64),
    )
    .await
    .expect("the test write completes");
}

#[tokio::test]
async fn add_test_key_accepts_a_cache_without_redis() {
    add_test_key(Arc::new(local_cache()), Arc::new(|| 1_700_000_000.0_f64))
        .await
        .expect("the test write completes");
}

#[tokio::test]
async fn a_flush_size_past_the_envelope_range_is_reported() {
    let mut cache = local_cache();
    cache.redis_flush_size = Some(usize::MAX);
    let error = add_test_key(Arc::new(cache), Arc::new(|| 1_700_000_000.0_f64))
        .await
        .expect_err("the flush size does not fit an i64");
    assert!(
        error
            .to_string()
            .contains("does not fit the cache envelope")
    );
    assert!(error.source().is_some());
}
