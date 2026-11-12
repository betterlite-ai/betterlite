use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::sink::PostHogSink;

// LiteLLM registers _flush_on_exit with atexit in __init__; betterlite hooks the server's shutdown instead
#[must_use]
pub fn spawn_exit_flush(
    sink: PostHogSink,
    shutdown: CancellationToken,
    tasks: &TaskTracker,
) -> bool {
    let Ok(handle) = Handle::try_current() else {
        return false;
    };
    let _task = tasks.spawn_on(
        async move {
            shutdown.cancelled().await;
            sink.flush_on_exit().await;
        },
        &handle,
    );
    true
}
