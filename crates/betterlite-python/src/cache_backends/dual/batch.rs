use std::sync::{Arc, Mutex};

use betterlite_cache::batch_reads::{BatchReads, DeclaredBatchRead};
use betterlite_models::decisions::Json;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use tokio_util::task::TaskTracker;

use super::{AsyncAnswer, MemoryDual, answer_json};
use crate::cache_backends::redis::NativeRedis;

// The batch a caller hands to declare_batch_get: its reads ride one pipeline round trip on that Redis tier.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeRedisBatch {
    pub(super) batch: Arc<RedisBatch>,
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDeferred {
    pub(super) result: Mutex<Option<BatchResult<()>>>,
}

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeDeclaredBatchRead {
    pub(super) cache: Arc<MemoryDual>,
    pub(super) declared: Mutex<Option<DeclaredBatchRead>>,
}

#[pymethods]
impl NativeRedisBatch {
    #[new]
    // ast-grep-ignore: no-one-line-functions - Python calls this constructor, so there is no Rust caller to inline into
    fn new(redis: PyRef<'_, NativeRedis>) -> Self {
        let tracker = TaskTracker::new();
        let batch = RedisBatch::new(Arc::clone(&redis.cache), "request_redis_batch", tracker);
        drop(redis);
        Self {
            batch: Arc::new(batch),
        }
    }
}

#[pymethods]
impl NativeDeferred {
    fn async_wait<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let result = take_slot(&self.result)
            .ok_or_else(|| PyRuntimeError::new_err("deferred cache result was already consumed"))?;
        future_into_py(py, async move {
            result
                .await
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })
    }

    pub(super) fn cancel(&self) {
        if let Some(result) = take_slot(&self.result) {
            result.cancel();
        }
    }
}

#[pymethods]
impl NativeDeclaredBatchRead {
    fn async_resolve<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let declared = take_slot(&self.declared)
            .ok_or_else(|| PyRuntimeError::new_err("declared batch read was already consumed"))?;
        future_into_py(py, resolve(Arc::clone(&self.cache), declared))
    }
}

fn take_slot<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    match slot.lock() {
        Ok(mut held) => held.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

async fn resolve(
    cache: Arc<MemoryDual>,
    declared: DeclaredBatchRead,
) -> PyResult<Vec<Option<String>>> {
    let values = BatchReads::async_resolve_batch_get(&*cache, declared)
        .await
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    values.into_iter().map(answer_json).collect()
}

pub(super) fn encode(values: Option<Vec<Option<Json>>>) -> PyResult<Option<Vec<Option<String>>>> {
    values
        .map(|entries| entries.into_iter().map(answer_json).collect())
        .transpose()
}

pub(super) async fn read_shared(
    owned: Vec<(Arc<MemoryDual>, Vec<String>)>,
) -> PyResult<Vec<Option<Vec<Option<String>>>>> {
    let refs = owned
        .iter()
        .map(|(cache, keys)| (&**cache, keys.as_slice()))
        .collect::<Vec<_>>();
    BatchReads::async_batch_get_cache_shared(&refs)
        .await
        .into_iter()
        .map(encode)
        .collect()
}

pub(super) fn deferred(
    py: Python<'_>,
    result: Option<BatchResult<()>>,
) -> PyResult<Option<Py<NativeDeferred>>> {
    result
        .map(|pending| {
            Py::new(
                py,
                NativeDeferred {
                    result: Mutex::new(Some(pending)),
                },
            )
        })
        .transpose()
}

mod tests;
