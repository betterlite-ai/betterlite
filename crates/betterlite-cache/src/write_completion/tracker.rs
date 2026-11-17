use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_context::call_context::{CallContext, CallState};
use betterlite_context::local_ttl_cache::CacheError;
use tokio::task::JoinHandle;
use tokio::time::timeout;
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

// The handle of a pending write, so its owner can see how the write ended.
pub struct CacheWriteHandle {
    pub task: JoinHandle<CacheWriteOutcome>,
}

impl CacheWriteTracker {
    // LiteLLM's create_cache_write_task: the write starts now, in the post-response phase, and the factory gets that phase to hand to the tasks it starts
    #[must_use]
    pub fn spawn<Metadata, Client, Write>(
        &self,
        context: &CallContext<'_, Metadata, Client>,
        write: Write,
    ) -> CacheWriteHandle
    where
        Write: Fn(CallState) -> CacheWriteFuture + Send + Sync + 'static,
    {
        let call = *context.post_response().view().state;
        let task = self.tasks.spawn(complete_despite_cancellation(
            write,
            call,
            self.cancellation.clone(),
        ));
        CacheWriteHandle { task }
    }

    // a wait() only completes once the tracker is closed, so close it and let the tracked writes finish
    pub async fn wait_idle(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

// LiteLLM's _complete_cache_write_despite_cancellation: a cancelled write runs the factory once more, for five seconds at most
async fn complete_despite_cancellation<Write>(
    write: Write,
    call: CallState,
    cancellation: CancellationToken,
) -> CacheWriteOutcome
where
    Write: Fn(CallState) -> CacheWriteFuture,
{
    if cancellation.is_cancelled() {
        flush_pending_write(&write, call).await;
        return CacheWriteOutcome::Cancelled;
    }
    let outcome = tokio::select! {
        biased;
        () = cancellation.cancelled() => None,
        result = write(call) => Some(result),
    };
    match outcome {
        Some(Ok(())) => CacheWriteOutcome::Completed,
        Some(Err(error)) => CacheWriteOutcome::Failed { error },
        None => {
            flush_pending_write(&write, call).await;
            CacheWriteOutcome::Cancelled
        }
    }
}

// The write the event loop interrupted is redone from its factory, never resumed, and a failure here is logged and dropped.
async fn flush_pending_write<Write>(write: &Write, call: CallState)
where
    Write: Fn(CallState) -> CacheWriteFuture,
{
    match timeout(CACHE_WRITE_FLUSH_TIMEOUT, write(call)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(
            "LiteLLM Cache: pending cache write failed during event loop shutdown: {error}"
        ),
        Err(elapsed) => tracing::warn!(
            "LiteLLM Cache: pending cache write failed during event loop shutdown: {elapsed}"
        ),
    }
}
