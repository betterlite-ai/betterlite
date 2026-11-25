use std::sync::{Arc, Mutex};

use betterlite_cache::batch_reads::{BatchReads, DeclaredBatchRead};
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;

use super::{AsyncAnswer, MemoryDual, answer_json};

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
impl NativeDeferred {
    fn async_wait<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        let result = take_result(&self.result, "deferred cache result was already consumed")?;
        future_into_py(py, async move {
            result
                .await
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })
    }

    pub(super) fn cancel(&self) {
        match self.result.lock() {
            Ok(mut held) => {
                if let Some(result) = held.take() {
                    result.cancel();
                }
            }
            Err(poisoned) => {
                if let Some(result) = poisoned.into_inner().take() {
                    result.cancel();
                }
            }
        }
    }
}

#[pymethods]
impl NativeDeclaredBatchRead {
    fn async_resolve<'py>(&self, py: Python<'py>) -> AsyncAnswer<'py> {
        take_result(&self.declared, "declared batch read was already consumed")
            .and_then(|declared| future_into_py(py, resolve(Arc::clone(&self.cache), declared)))
    }
}

fn take_result<T>(slot: &Mutex<Option<T>>, consumed: &str) -> PyResult<T> {
    let value = match slot.lock() {
        Ok(mut held) => held.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    value.ok_or_else(|| PyRuntimeError::new_err(consumed.to_owned()))
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
