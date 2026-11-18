use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tokio_util::task::TaskTracker;

use super::billing::bill_partial;
use super::log_text::call_id_text;
use super::marks::{mark_details_disconnected, mark_request_disconnected};
use super::ports::{CleanupError, RequestLogging, SlotRelease};
use crate::sse_data_generator::types::{CleanupOutcome, StreamCleanup};

pub struct DisconnectCleanup<Key> {
    pub logging: Arc<dyn RequestLogging>,
    pub slots: Arc<dyn SlotRelease<Key>>,
    pub tracker: TaskTracker,
    pub disable_streaming_logging: bool,
}

impl<Key: Send + Sync> DisconnectCleanup<Key> {
    pub async fn run_cleanup(&self, mut outcome: CleanupOutcome<Key>) -> Result<(), CleanupError> {
        if outcome.client_disconnected {
            self.settle_disconnect(&mut outcome).await?;
        }
        self.invalidate_if_unfinished(&outcome).await
    }

    async fn settle_disconnect(
        &self,
        outcome: &mut CleanupOutcome<Key>,
    ) -> Result<(), CleanupError> {
        self.record_disconnect(&mut outcome.request_data);
        let armed = self.logging.deferred_logging_armed(&outcome.request_data);
        self.logging.fire_deferred_logging(&outcome.request_data);
        let owns = armed
            || bill_partial(
                self.logging.as_ref(),
                self.disable_streaming_logging,
                &outcome.request_data,
                outcome.response.as_ref(),
            )
            .await;
        if owns {
            return Ok(());
        }
        self.slots.release_on_disconnect(&outcome.key).await
    }

    fn record_disconnect(&self, request_data: &mut JsonObject) {
        self.logging
            .edit_call_details(request_data, mark_details_disconnected);
        mark_request_disconnected(request_data);
        tracing::debug!(
            "Recorded streaming client disconnect with error_code=499 for litellm_call_id={}",
            call_id_text(request_data)
        );
    }

    async fn invalidate_if_unfinished(
        &self,
        outcome: &CleanupOutcome<Key>,
    ) -> Result<(), CleanupError> {
        if outcome.stream_completed {
            return Ok(());
        }
        let Some(details) = self.logging.call_details(&outcome.request_data) else {
            return Ok(());
        };
        if !self.logging.baseline_cache_active(&outcome.request_data) {
            return Ok(());
        }
        if matches!(
            details.get("prompt_cache_response_complete"),
            Some(Json::Bool(true))
        ) {
            return Ok(());
        }
        self.logging
            .invalidate_baseline(&outcome.request_data, "incomplete_response", true)
            .await
    }
}

impl<Key> StreamCleanup<Key> for DisconnectCleanup<Key>
where
    Key: Send + Sync + 'static,
{
    fn finalize(&self, outcome: CleanupOutcome<Key>) {
        let cleanup = DisconnectCleanup {
            logging: Arc::clone(&self.logging),
            slots: Arc::clone(&self.slots),
            tracker: self.tracker.clone(),
            disable_streaming_logging: self.disable_streaming_logging,
        };
        let _task = self.tracker.spawn(async move {
            if let Err(error) = cleanup.run_cleanup(outcome).await {
                tracing::error!("stream cleanup failed: {error}");
            }
        });
    }
}
