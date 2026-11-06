use std::error::Error;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};

use thiserror::Error;
use tokio::task::JoinError;
use tokio::time::{Instant, error::Elapsed, timeout_at};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

#[derive(Debug, Error)]
pub enum CleanupError<Source> {
    #[error("the caller was cancelled while cleanup ran")]
    Cancelled,
    #[error("cleanup passed its deadline")]
    TimedOut(#[source] Elapsed),
    #[error("cleanup failed")]
    Operation(#[source] Source),
    #[error("the cleanup task ended without a result")]
    Task(#[source] JoinError),
}

// The operation runs in a tracked task, so dropping this future or cancelling the caller never abandons it
pub async fn run_bounded_cleanup<Output, Source, Operation>(
    tracker: &TaskTracker,
    caller_cancelled: &CancellationToken,
    deadline: Instant,
    operation: Operation,
) -> Result<Output, CleanupError<Source>>
where
    Operation: Future<Output = Result<Output, Source>> + Send + 'static,
    Output: Send + 'static,
    Source: Error + Send + 'static,
{
    let mut task = tracker.spawn(timeout_at(deadline, operation));
    let mut cancellation = pin!(caller_cancelled.cancelled());
    let mut interrupted = false;
    let joined = poll_fn(|context| {
        if !interrupted && cancellation.as_mut().poll(context).is_ready() {
            interrupted = true;
        }
        Pin::new(&mut task).poll(context)
    })
    .await;
    if interrupted {
        return Err(CleanupError::Cancelled);
    }
    match joined {
        Ok(Ok(Ok(output))) => Ok(output),
        Ok(Ok(Err(source))) => Err(CleanupError::Operation(source)),
        Ok(Err(elapsed)) => Err(CleanupError::TimedOut(elapsed)),
        Err(join_error) => Err(CleanupError::Task(join_error)),
    }
}
