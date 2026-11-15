use std::sync::Arc;
use std::sync::PoisonError;

use tokio::time::timeout;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::GcsBucketLogger;
use super::async_send_behaviors::async_send_batch;

pub async fn flush_queue<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>)
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
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
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    loop {
        if timeout(logger.settings.flush_interval, logger.stopped.cancelled())
            .await
            .is_ok()
        {
            return;
        }
        let seconds = logger.settings.flush_interval.as_secs();
        tracing::debug!(
            seconds,
            "GCS Bucket periodic flush after {} seconds",
            seconds
        );
        flush_queue(logger.as_ref()).await;
        tracing::debug!("GCS Bucket periodic flush completed");
    }
}

mod tests;
