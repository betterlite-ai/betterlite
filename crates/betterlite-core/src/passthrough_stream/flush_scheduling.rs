use std::sync::Arc;

use tokio::runtime::Handle;

use super::async_lifecycle::AsyncPassthroughStream;
use super::flush_errors::report_flush_outcome;
use super::logging::{SpendLogging, SyncFlushExecutor};
use super::spend_collection::SpendSink;
use super::sync_lifecycle::PassthroughStreamingResponse;

mod tests;

pub(super) fn start_async_flush<S, L>(stream: &mut AsyncPassthroughStream<S, L>)
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
{
    if stream.flush_scheduled {
        return;
    }
    let Some((collector, chunk_count)) = stream.spend.take_flush() else {
        return;
    };
    stream.flush_scheduled = true;
    let Ok(handle) = Handle::try_current() else {
        tracing::error!(
            chunks = chunk_count,
            "Failed to schedule passthrough spend-tracking flush; {chunk_count} collected chunks dropped: no async runtime is running"
        );
        return;
    };
    let logging = Arc::clone(&stream.logging);
    stream.background_tasks.spawn_on(
        async move {
            report_flush_outcome(
                logging.async_flush_collected_chunks(collector).await,
                chunk_count,
            );
        },
        &handle,
    );
}

pub(super) fn start_sync_flush<S, L, E>(stream: &mut PassthroughStreamingResponse<S, L, E>)
where
    S: SpendSink,
    L: SpendLogging<S::Logged>,
    E: SyncFlushExecutor,
{
    if stream.flush_scheduled {
        return;
    }
    let Some((collector, chunk_count)) = stream.spend.take_flush() else {
        return;
    };
    stream.flush_scheduled = true;
    let logging = Arc::clone(&stream.logging);
    let job: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        report_flush_outcome(logging.flush_collected_chunks(collector), chunk_count);
    });
    if let Err(error) = stream.executor.submit(job) {
        tracing::error!(
            chunks = chunk_count,
            "Failed to schedule passthrough spend-tracking flush; {chunk_count} collected chunks dropped: {error}"
        );
    }
}
