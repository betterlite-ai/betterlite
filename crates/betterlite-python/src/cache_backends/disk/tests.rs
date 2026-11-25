#![cfg(test)]

use std::sync::Arc;

use betterlite_cache::disk::DiskCache;
use betterlite_context::local_ttl_cache::CacheValue;
use pyo3_async_runtimes::tokio::get_runtime;
use tempfile::TempDir;

use super::*;
use crate::cache_backends::convert;

fn native_disk(directory: &TempDir) -> NativeDisk {
    Python::initialize();
    let clock = Arc::new(convert::wall_clock);
    let cache = DiskCache::open(Some(directory.path()), clock).unwrap();
    NativeDisk {
        cache: Arc::new(cache),
    }
}

#[test]
fn backends_disk_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let native = native_disk(&directory);
    native.set("key", r#"{"a":1}"#, None).unwrap();
    assert_eq!(native.get("key").unwrap(), Some(r#"{"a":1}"#.to_owned()));
    assert_eq!(native.get("missing").unwrap(), None);
    native.delete(vec!["key".to_owned()]).unwrap();
    assert_eq!(native.get("key").unwrap(), None);
    native.set("text", "\"x\"", None).unwrap();
    native.flush().unwrap();
    assert_eq!(native.get("text").unwrap(), None);
}

#[test]
fn backends_disk_counts_increments() {
    let directory = tempfile::tempdir().unwrap();
    let native = native_disk(&directory);
    assert_eq!(native.increment("count", 2_i64, None).unwrap(), "2");
    assert_eq!(native.increment("count", 3_i64, None).unwrap(), "5");
}

#[test]
fn backends_disk_ttl_answers_none() {
    let directory = tempfile::tempdir().unwrap();
    let native = native_disk(&directory);
    native.set("key", "1", Some(60.0_f64)).unwrap();
    assert_eq!(native.ttl("key").unwrap(), None);
}

#[test]
fn backends_disk_ping_and_close() {
    let directory = tempfile::tempdir().unwrap();
    let native = native_disk(&directory);
    assert!(native.ping().unwrap());
    native.close().unwrap();
}

#[test]
fn backends_disk_answer_text_encodes() {
    let value = CacheValue::Text("x".to_owned());
    assert_eq!(answer_text(Some(value)).unwrap(), Some("\"x\"".to_owned()));
    assert_eq!(answer_text(None).unwrap(), None);
}

#[test]
fn backends_disk_count_adds() {
    let directory = tempfile::tempdir().unwrap();
    let native = native_disk(&directory);
    let counted = get_runtime()
        .block_on(count(
            Arc::clone(&native.cache),
            "count".to_owned(),
            2_i64,
            None,
        ))
        .unwrap();
    assert_eq!(counted, "2");
}
