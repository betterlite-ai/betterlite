use std::sync::{Arc, PoisonError};

use crate::batch::logger::BatchQueue;

use super::batch::{
    CredentialGroup, FAILURE_STATUS, OK_STATUS, PostHogBatch, PostHogSendError, QueuedEvent,
    group_events, send_group,
};
use super::delivery::Transport;
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;

#[derive(Clone)]
pub struct PostHogSink {
    pub queue: BatchQueue<QueuedEvent>,
    pub transport: Arc<dyn Transport>,
    pub url_check: Arc<dyn UploadUrlCheck>,
    pub mock_mode: bool,
}

impl PostHogSink {
    pub fn queued_depth(&self) -> usize {
        self.queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub async fn flush(&self) {
        if self.queued_depth() == 0 {
            return;
        }
        let _report = self.queue.flush(&PostHogBatch { sink: self }).await;
    }

    pub async fn send_one(&self, queued: QueuedEvent) {
        let group = CredentialGroup {
            api_key: queued.api_key,
            api_url: queued.api_url,
            request_url: queued.request_url,
            events: vec![queued.event],
        };
        match send_group(
            self.transport.as_ref(),
            self.mock_mode,
            self.url_check.as_ref(),
            &group,
        )
        .await
        {
            Ok(OK_STATUS) => tracing::debug!("PostHog: Sync event successfully sent"),
            Err(PostHogSendError::RefusedUrl(refused)) => {
                tracing::warn!(error = %refused, "PostHog: refused a request-supplied PostHog URL");
            }
            Ok(status) => tracing::error!(status, "PostHog Sync Layer Error"),
            Err(error) => tracing::error!(%error, "PostHog Sync Layer Error"),
        }
    }

    // PostHogLogger._flush_on_exit: any failed post leaves the whole queue for a later flush
    pub async fn flush_on_exit(&self) {
        let _flush = self.queue.flush_lock.lock().await;
        let pending = self.queue.pending();
        if pending.is_empty() {
            return;
        }
        let queued = pending.len();
        for group in group_events(pending) {
            match send_group(
                self.transport.as_ref(),
                self.mock_mode,
                self.url_check.as_ref(),
                &group,
            )
            .await
            {
                Ok(OK_STATUS) => {}
                Err(PostHogSendError::RefusedUrl(refused)) => {
                    tracing::warn!(error = %refused, "PostHog: refused a request-supplied PostHog URL");
                }
                // LiteLLM's raise_for_status ignores a status under 400, so the flush goes on
                Ok(status) if status < FAILURE_STATUS => {
                    tracing::error!(status, "PostHog: Failed to flush on exit");
                }
                Ok(status) => {
                    tracing::error!(status, "PostHog: Error flushing events on exit");
                    return;
                }
                Err(error) => {
                    tracing::error!(%error, "PostHog: Error flushing events on exit");
                    return;
                }
            }
        }
        self.drain_flushed(queued);
        tracing::debug!("PostHog: Successfully flushed events on exit");
    }

    fn drain_flushed(&self, queued: usize) {
        let mut entries = self
            .queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let flushed = queued.min(entries.len());
        entries.drain(..flushed).for_each(drop);
    }
}
