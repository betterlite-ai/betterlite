use std::fmt::Display;
use std::future::Future;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

// the caller owns the tracker, so spawned work outlives the stream that requested it
pub fn spawn_logging_task<Work, Failure>(
    pending: &TaskTracker,
    task_name: &str,
    work: Work,
) -> Option<JoinHandle<()>>
where
    Work: Future<Output = Result<(), Failure>> + Send + 'static,
    Failure: Display + Send + 'static,
{
    let handle = Handle::try_current().ok()?;
    let name = task_name.to_owned();
    let task = pending.spawn_on(
        async move {
            if let Err(failure) = work.await {
                tracing::error!("{name} failed: {failure}");
            }
        },
        &handle,
    );
    Some(task)
}
