pub mod health;
pub mod read_only;
pub mod replica_fallback;
pub mod views;
pub mod watchdog;

mod tests;

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};

use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;
use tracing::warn;

pub type LifecycleFuture<'borrow, Output> = Pin<Box<dyn Future<Output = Output> + Send + 'borrow>>;

// one slot per lifecycle loop: start reports false while a task runs, stop returns whether it had one
pub struct TrackedTask {
    pub handle: Mutex<Option<JoinHandle<()>>>,
}

impl TrackedTask {
    #[must_use]
    pub fn start_with<Task>(&self, tasks: &TaskTracker, task: Task) -> bool
    where
        Task: Future<Output = ()> + Send + 'static,
    {
        let mut handle = self.handle.lock().unwrap_or_else(PoisonError::into_inner);
        if handle.is_some() {
            return false;
        }
        *handle = Some(tasks.spawn(task));
        true
    }

    #[must_use]
    pub async fn stop(&self) -> bool {
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(handle) = handle else {
            return false;
        };
        handle.abort();
        if let Err(error) = handle.await
            && !error.is_cancelled()
        {
            warn!("a database lifecycle task ended with an error: {error}");
        }
        true
    }
}

// LiteLLM tells a dead connection from a caller's error and a planned replacement from a crash; the port answers both here.
pub trait DatabaseFailure: Error + Send + Sync + 'static {
    fn is_connection_failure(&self) -> bool {
        true
    }

    fn is_infrastructure_failure(&self) -> bool {
        true
    }
}

pub trait LifecycleDatabase: Send + Sync {
    type Error: DatabaseFailure;
    type Target: health::HealthTarget<Error = Self::Error>;

    fn probe_target(&self) -> Self::Target;

    fn writer_is_read_only(&self) -> LifecycleFuture<'_, Result<bool, Self::Error>>;

    fn writer_unavailable(&self) -> bool;

    fn spend_logs_row_count(&self) -> LifecycleFuture<'_, Result<Option<i64>, Self::Error>>;
}

pub trait HealthFailureReporter: Send + Sync {
    fn report(&self, failure: health::HealthFailure) -> LifecycleFuture<'_, ()>;
}

pub trait ProxyStateSink: Send + Sync {
    fn set_spend_logs_row_count(&self, row_count: i64);
}
