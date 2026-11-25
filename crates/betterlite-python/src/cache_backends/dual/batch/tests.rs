#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_cache::batch_reads::PendingBatchRead;
use tokio_util::task::TaskTracker;

use super::*;
use crate::cache_backends::dual::tests::{dead_tier, native_dual};
use pyo3_async_runtimes::tokio::get_runtime;

#[test]
fn batch_results_are_consumed_once() {
    let slot = Mutex::new(Some(7_u8));
    assert_eq!(take_slot(&slot), Some(7_u8));
    assert_eq!(take_slot(&slot), None);
}

#[test]
fn batch_encodes_misses_and_empty_reads() {
    assert_eq!(encode(None).unwrap(), None);
    assert_eq!(encode(Some(Vec::new())).unwrap(), Some(Vec::new()));
    assert_eq!(
        encode(Some(vec![Some(Json::Bool(true)), None])).unwrap(),
        Some(vec![Some("true".to_owned()), None])
    );
}

#[test]
fn batch_reads_shared() {
    let native = native_dual(None);
    Python::attach(|py| native.set(py, "key", "true", None).unwrap());
    let reads = vec![
        (Arc::clone(&native.cache), vec!["key".to_owned()]),
        (Arc::clone(&native.cache), vec!["missing".to_owned()]),
    ];
    assert_eq!(
        get_runtime().block_on(read_shared(reads)).unwrap(),
        vec![Some(vec![Some("true".to_owned())]), Some(vec![None])]
    );
}

fn supplied_batch(py: Python<'_>) -> Py<NativeRedisBatch> {
    let batch = RedisBatch::new(dead_tier(), "request_redis_batch", TaskTracker::new());
    let supplied = NativeRedisBatch {
        batch: Arc::new(batch),
    };
    Py::new(py, supplied).unwrap()
}

#[test]
fn batch_declares_the_read_on_the_supplied_batch() {
    let native = native_dual(Some(dead_tier()));
    Python::attach(|py| {
        let batch = supplied_batch(py);
        let keys = vec!["key".to_owned()];
        let declared = native
            .declare_batch_get(py, keys, batch.borrow(py))
            .unwrap();
        assert_eq!(batch.borrow(py).batch.pending(), 1);
        let held = declared.borrow(py);
        let slot = held.declared.lock().unwrap();
        let read = slot.as_ref().unwrap();
        assert_eq!(read.pending.redis_keys, vec!["key".to_owned()]);
        assert!(read.result.is_some());
        drop(slot);
    });
}

#[test]
fn batch_declaration_without_redis_declares_nothing() {
    let native = native_dual(None);
    Python::attach(|py| {
        let batch = supplied_batch(py);
        let keys = vec!["key".to_owned()];
        let declared = native
            .declare_batch_get(py, keys, batch.borrow(py))
            .unwrap();
        assert_eq!(batch.borrow(py).batch.pending(), 0);
        let held = declared.borrow(py);
        let slot = held.declared.lock().unwrap();
        assert!(slot.as_ref().unwrap().result.is_none());
        drop(slot);
        assert!(deferred(py, None).unwrap().is_none());
    });
}

#[test]
fn batch_resolves_declared_memory_values() {
    let native = native_dual(None);
    let declared = DeclaredBatchRead {
        keys: vec!["key".to_owned(), "missing".to_owned()],
        pending: PendingBatchRead {
            keys: vec!["key".to_owned(), "missing".to_owned()],
            result: vec![Some(Json::Bool(true)), None],
            redis_keys: Vec::new(),
            previous_access_times: BTreeMap::new(),
        },
        result: None,
    };
    assert_eq!(
        get_runtime()
            .block_on(resolve(Arc::clone(&native.cache), declared))
            .unwrap(),
        vec![Some("true".to_owned()), None]
    );
}
