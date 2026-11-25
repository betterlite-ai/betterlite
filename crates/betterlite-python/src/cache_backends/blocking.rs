use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use tokio_util::task::TaskTracker;

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

// The blocking work starts only once the Python future exists, and the owner's tracker keeps it until close drains it.
pub(super) fn awaitable<Work, Failure>(
    py: Python<'_>,
    tasks: TaskTracker,
    work: Work,
) -> PyResult<Bound<'_, PyAny>>
where
    Work: FnOnce() -> Result<(), Failure> + Send + 'static,
    Failure: Send + 'static,
    BackendHandleError: From<Failure>,
{
    none_future(py, async move {
        let outcome = tasks
            .spawn_blocking(move || work().map_err(BackendHandleError::from))
            .await
            .map_err(|source| BackendHandleError::Join { source })?;
        outcome.map_err(PyErr::from)
    })
}

// Close waits for the blocking work still running, then lets the Redis connection go.
pub(super) async fn close(tasks: TaskTracker, tier: Option<Arc<RedisCache>>) -> PyResult<()> {
    tasks.close();
    tasks.wait().await;
    if let Some(redis) = tier {
        redis.admin.disconnect().await;
    }
    Ok(())
}

mod tests;
