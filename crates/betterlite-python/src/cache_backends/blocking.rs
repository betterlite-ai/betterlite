use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::error::BackendHandleError;

// A bare () would reach Python as an empty tuple, so the finished call answers None.
pub(super) fn none_future<Work>(py: Python<'_>, work: Work) -> PyResult<Bound<'_, PyAny>>
where
    Work: Future<Output = PyResult<()>> + Send + 'static,
{
    future_into_py(py, async move {
        work.await?;
        Ok(None::<()>)
    })
}

pub(super) fn awaitable<Work, Failure>(py: Python<'_>, work: Work) -> PyResult<Bound<'_, PyAny>>
where
    Work: FnOnce() -> Result<(), Failure> + Send + 'static,
    Failure: Send + 'static,
    BackendHandleError: From<Failure>,
{
    let task = get_runtime().spawn_blocking(move || work().map_err(BackendHandleError::from));
    none_future(py, async move {
        let outcome = task
            .await
            .map_err(|source| BackendHandleError::Join { source })?;
        outcome.map_err(PyErr::from)
    })
}

mod tests;
