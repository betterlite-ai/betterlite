use super::logging::SpendLogging;
use super::spend_collection::BoxedCollector;

mod tests;

// LiteLLM's created task can fail where nobody awaits it; here the failure is said out loud and the relay goes on
pub(super) async fn run_async_flush<L, Logged>(
    logging: &L,
    collector: BoxedCollector<Logged>,
    chunk_count: usize,
) where
    L: SpendLogging<Logged> + ?Sized,
    Logged: Send + 'static,
{
    if let Err(error) = logging.async_flush_collected_chunks(collector).await {
        tracing::error!(
            chunks = chunk_count,
            %error,
            "the passthrough spend-tracking flush failed; {chunk_count} collected chunks dropped"
        );
    }
}

// The executor job for the blocking relay: same best-effort rule as the async task
pub(super) fn run_sync_flush<L, Logged>(
    logging: &L,
    collector: BoxedCollector<Logged>,
    chunk_count: usize,
) where
    L: SpendLogging<Logged> + ?Sized,
    Logged: Send + 'static,
{
    if let Err(error) = logging.flush_collected_chunks(collector) {
        tracing::error!(
            chunks = chunk_count,
            %error,
            "the passthrough spend-tracking flush failed; {chunk_count} collected chunks dropped"
        );
    }
}
