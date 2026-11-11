use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use super::payload::metering_event;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::finops::metering::lago::LagoLogger;
use crate::finops::metering::transport::MeteringSender;

pub(super) struct LagoAdapter<Sender: MeteringSender> {
    pub(super) logger: Arc<LagoLogger<Sender>>,
    pub(super) tracker: TaskTracker,
}

// LiteLLM's sync success hook is sync and posts on the blocking client, so it goes onto the owned tracker
#[async_trait]
impl<Sender: MeteringSender + 'static> CallbackLogger for LagoAdapter<Sender> {
    fn callback_name(&self) -> &'static str {
        "lago"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = metering_event(&payload);
            if let Err(error) = logger.log_success_event(&call).await {
                tracing::warn!(%error, "the lago success event could not be logged");
            }
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let call = metering_event(&event.payload);
        self.logger
            .async_log_success_event(&call)
            .await
            .map_err(|error| LoggerError::Delivery {
                callback: "lago".to_owned(),
                message: error.to_string(),
            })
    }
}
