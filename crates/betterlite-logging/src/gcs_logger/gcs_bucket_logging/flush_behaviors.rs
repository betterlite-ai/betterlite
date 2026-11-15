use std::fmt::Debug;
use std::sync::Arc;
use std::sync::PoisonError;

use tokio::time::timeout;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::GcsBucketLogger;
use super::async_send_behaviors::async_send_batch;

pub async fn flush_queue<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>)
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    async_send_batch(logger).await;
    let now = logger.clock.now();
    *logger
        .last_flush_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = now;
}

// LiteLLM sleeps flush_interval, flushes and repeats; betterlite ends the loop on shutdown
pub async fn periodic_flush<Auth, Http>(logger: Arc<GcsBucketLogger<Auth, Http>>)
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let interval = logger.settings.flush_interval;
    while timeout(interval, logger.stopped.cancelled()).await.is_err() {
        tracing::debug!(
            seconds = interval.as_secs(),
            "GCS Bucket periodic flush after {} seconds",
            interval.as_secs()
        );
        flush_queue(logger.as_ref()).await;
        tracing::debug!("GCS Bucket periodic flush completed");
    }
}

mod tests;
