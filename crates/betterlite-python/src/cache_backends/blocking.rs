use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use tokio::task::JoinSet;

use super::error::{self, BackendHandleError};

pub(super) fn into_py<Work, Failure>(py: Python<'_>, work: Work) -> PyResult<Bound<'_, PyAny>>
where
    Work: FnOnce() -> Result<(), Failure> + Send + 'static,
    Failure: Send + 'static,
    BackendHandleError: From<Failure>,
{
    future_into_py(py, async move {
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(work);
        let outcome = tasks
            .join_next()
            .await
            .ok_or_else(|| PyRuntimeError::new_err("flush task was not started"))?
            .map_err(BackendHandleError::from)?;
        error::backend_err(outcome)
    })
}

mod tests;
