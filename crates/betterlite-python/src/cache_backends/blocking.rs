use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::error::BackendHandleError;

pub(super) fn awaitable<Work, Failure>(py: Python<'_>, work: Work) -> PyResult<Bound<'_, PyAny>>
where
    Work: FnOnce() -> Result<(), Failure> + Send + 'static,
    Failure: Send + 'static,
    BackendHandleError: From<Failure>,
{
    let task = get_runtime().spawn_blocking(move || work().map_err(BackendHandleError::from));
    future_into_py(py, async move {
        let outcome = task
            .await
            .map_err(|source| BackendHandleError::Join { source })?;
        outcome.map_err(PyErr::from)
    })
}

mod tests;
