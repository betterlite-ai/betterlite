#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE, Ttl};
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;
use pyo3_async_runtimes::tokio::{get_runtime, into_future, run};
use serde_json::Number;
use tokio::time::sleep;

use super::super::redis::open;
use super::*;
use crate::cache_backends::convert;
use crate::cache_backends::settings::RedisSettings;

const ITEM_KIB: i64 = 1024;
const DEAD_TIER: &str = r#"{"overrides": {"host": "127.0.0.1", "port": "1", "socket_timeout": "0.1", "socket_connect_timeout": "0.1"}}"#;

pub(super) fn native_dual(redis: Option<Arc<RedisCache>>) -> NativeDual {
    Python::initialize();
    let clock = Arc::new(convert::wall_clock);
    let ops = Arc::new(LocalMemoryOps::new(ITEM_KIB, clock).unwrap());
    let settings = DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    let cache = DualCache::new(ops, redis.clone(), settings);
    NativeDual {
        cache: Arc::new(cache),
        redis,
        memory_ttl: None,
        tasks: TaskTracker::new(),
    }
}

// Nothing listens on port 1, so the tier is only there to make the dual cache defer its writes.
pub(super) fn dead_tier() -> Arc<RedisCache> {
    let parsed: RedisSettings = convert::parse_settings(DEAD_TIER).unwrap();
    Python::attach(|py| open(py, parsed)).unwrap().cache
}

#[test]
fn backends_dual_round_trip() {
    let native = native_dual(None);
    Python::attach(|py| {
        native.set(py, "key", r#"{"a":1}"#, None).unwrap();
        assert_eq!(
            native.get(py, "key").unwrap(),
            Some(r#"{"a":1}"#.to_owned())
        );
        assert_eq!(native.get(py, "missing").unwrap(), None);
        native.delete(py, vec!["key".to_owned()]).unwrap();
        assert_eq!(native.get(py, "key").unwrap(), None);
        native.set(py, "text", "\"x\"", None).unwrap();
        native.flush(py).unwrap();
        assert_eq!(native.get(py, "text").unwrap(), None);
    });
}

#[test]
fn backends_dual_counts_increments() {
    let native = native_dual(None);
    Python::attach(|py| {
        assert_eq!(native.increment(py, "count", 2.0_f64, None).unwrap(), "2.0");
        assert_eq!(native.increment(py, "count", 2.0_f64, None).unwrap(), "4.0");
    });
}

#[test]
fn backends_dual_ttl_reads_the_remaining() {
    let native = native_dual(None);
    Python::attach(|py| {
        native.set(py, "key", "1", Some(60.0_f64)).unwrap();
        let remaining = native
            .ttl(py, "key")
            .unwrap()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(remaining > 0.0_f64);
        assert_eq!(native.ttl(py, "missing").unwrap(), None);
    });
}

#[test]
fn backends_dual_ping_without_redis() {
    let native = native_dual(None);
    Python::attach(|py| {
        assert!(native.ping(py).unwrap());
        native.close(py).unwrap();
    });
}

// Close returns only after the flush the tracker still holds has finished.
#[test]
fn backends_dual_close_drains_running_flushes() {
    let native = native_dual(None);
    let finished = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&finished);
    let running = native.tasks.spawn_on(
        async move {
            sleep(Duration::from_millis(200)).await;
            marker.store(true, Ordering::SeqCst);
        },
        get_runtime().handle(),
    );
    Python::attach(|py| native.close(py)).unwrap();
    assert!(finished.load(Ordering::SeqCst));
    assert!(running.is_finished());
}

#[test]
fn backends_dual_batch_reads_memory_values() {
    let native = native_dual(None);
    Python::attach(|py| {
        native.set(py, "present", "1", None).unwrap();
        assert_eq!(
            native
                .batch_get(py, vec!["present".to_owned(), "missing".to_owned()], false)
                .unwrap(),
            Some(vec![Some("1".to_owned()), None])
        );
    });
}

#[test]
fn backends_dual_pre_call_without_redis_is_not_deferred() {
    let native = native_dual(None);
    Python::attach(|py| {
        assert!(
            native
                .set_cache_pre_call(py, "key", "1", None)
                .unwrap()
                .is_none()
        );
        assert!(native.delete_cache_pre_call(py, "key").unwrap().is_none());
    });
}

#[test]
fn backends_dual_options_and_text_of_ttl() {
    let cache_options = options(Some(5.0_f64));
    assert_eq!(cache_options.ttl, Some(5.0_f64));
    assert!(!cache_options.local_only);
    assert!(!cache_options.refresh_ttl);
    assert_eq!(
        text_of_ttl(Some(Ttl::Seconds(7))).unwrap(),
        Some("7".to_owned())
    );
    assert_eq!(text_of_ttl(None).unwrap(), None);
}

#[test]
fn backends_dual_store_and_answer_json() {
    let native = native_dual(None);
    get_runtime()
        .block_on(store(
            Arc::clone(&native.cache),
            "key".to_owned(),
            Json::Bool(true),
            None,
        ))
        .unwrap();
    Python::attach(|py| assert_eq!(native.get(py, "key").unwrap(), Some("true".to_owned())));
    let number = Json::Number(Number::from(3_i64));
    assert_eq!(answer_json(Some(number)).unwrap(), Some("3".to_owned()));
    assert_eq!(answer_json(None).unwrap(), None);
}

#[test]
fn backends_dual_count_adds() {
    let native = native_dual(None);
    let counted = get_runtime()
        .block_on(count(
            Arc::clone(&native.cache),
            "count".to_owned(),
            2.0_f64,
            None,
            None,
        ))
        .unwrap();
    assert_eq!(counted, "2.0");
}

#[test]
fn backends_dual_cancel_drops_the_deferred_write() {
    let native = native_dual(Some(dead_tier()));
    Python::attach(|py| {
        let (_batches, deferred) = get_runtime()
            .block_on(request_redis_batch_scope(Duration::from_secs(1), async {
                native.set_cache_pre_call(py, "key", "1", None).unwrap()
            }));
        let pending = deferred.unwrap();
        assert!(pending.borrow(py).result.lock().unwrap().is_some());
        pending.borrow(py).cancel();
        assert!(pending.borrow(py).result.lock().unwrap().is_none());
    });
}

#[test]
fn backends_dual_pipeline_post_call_counts_in_memory() {
    let native = native_dual(None);
    let operations = vec![
        ("first".to_owned(), 2.0_f64, None),
        ("second".to_owned(), 3.0_f64, Some(60)),
        ("first".to_owned(), 1.0_f64, None),
    ];
    let work = async move {
        let pending =
            Python::attach(|py| into_future(native.async_pipeline_post_call(py, operations)?))?;
        let answer = pending.await?;
        Python::attach(|py| {
            assert!(answer.is_none(py));
            assert_eq!(native.increment(py, "first", 0.0_f64, None).unwrap(), "3.0");
            assert_eq!(
                native.increment(py, "second", 0.0_f64, None).unwrap(),
                "3.0"
            );
        });
        Ok(())
    };
    Python::attach(|py| run(py, work)).unwrap();
}
