use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_context::call_context::Phase;
use betterlite_context::local_ttl_cache::CacheError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

// LiteLLM's CACHE_WRITE_SHUTDOWN_FLUSH_TIMEOUT_SECONDS
pub const CACHE_WRITE_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

pub type CacheWriteFuture = Pin<Box<dyn Future<Output = Result<(), CacheError>> + Send + 'static>>;

#[derive(Debug, Eq, PartialEq)]
pub enum CacheWriteOutcome {
    Completed,
    Failed { error: CacheError },
    Cancelled,
}

// Every unfinished cache write the tracker started; a task leaves the tracker when it ends.
pub struct CacheWriteTracker {
    pub tasks: TaskTracker,
    pub cancellation: CancellationToken,
}

// A pending write, marked with the post-response phase it runs in.
pub struct CacheWriteHandle {
    pub phase: Phase,
    pub task: JoinHandle<CacheWriteOutcome>,
}

impl CacheWriteTracker {
    // LiteLLM's create_cache_write_task: the write starts now and the caller stops waiting for it
    #[must_use]
    pub fn spawn<Write>(&self, write: Write) -> CacheWriteHandle
    where
        Write: Fn() -> CacheWriteFuture + Send + Sync + 'static,
    {
        let task = self.tasks.spawn(complete_despite_cancellation(
            write,
            self.cancellation.clone(),
        ));
        CacheWriteHandle {
            phase: Phase::PostResponse,
            task,
        }
    }

    // wait covers only the tasks spawned before it runs, so re-check until the tracker is empty
    pub async fn wait_idle(&self) {
        while !self.tasks.is_empty() {
            self.tasks.wait().await;
        }
    }
}

// LiteLLM's _complete_cache_write_despite_cancellation: a cancelled write runs the factory once more, for five seconds at most
async fn complete_despite_cancellation<Write>(
    write: Write,
    cancellation: CancellationToken,
) -> CacheWriteOutcome
where
    Write: Fn() -> CacheWriteFuture,
{
    if cancellation.is_cancelled() {
        flush_pending_write(&write).await;
        return CacheWriteOutcome::Cancelled;
    }
    let outcome = tokio::select! {
        biased;
        () = cancellation.cancelled() => None,
        result = write() => Some(result),
    };
    match outcome {
        Some(Ok(())) => CacheWriteOutcome::Completed,
        Some(Err(error)) => CacheWriteOutcome::Failed { error },
        None => {
            flush_pending_write(&write).await;
            CacheWriteOutcome::Cancelled
        }
    }
}

// The write the event loop interrupted is redone from its factory, never resumed, and a failure here is logged and dropped.
async fn flush_pending_write<Write>(write: &Write)
where
    Write: Fn() -> CacheWriteFuture,
{
    match tokio::time::timeout(CACHE_WRITE_FLUSH_TIMEOUT, write()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(
            "LiteLLM Cache: pending cache write failed during event loop shutdown: {error}"
        ),
        Err(elapsed) => tracing::warn!(
            "LiteLLM Cache: pending cache write failed during event loop shutdown: {elapsed}"
        ),
    }
}
