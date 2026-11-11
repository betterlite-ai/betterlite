use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use super::payload::metering_event;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::finops::metering::clock::UtcClock;
use crate::finops::metering::openmeter::OpenMeterLogger;
use crate::finops::metering::openmeter::error::OpenMeterError;
use crate::finops::metering::transport::MeteringSender;

pub(super) struct OpenMeterAdapter<Sender: MeteringSender, Time: UtcClock> {
    pub(super) logger: Arc<OpenMeterLogger<Sender, Time>>,
    pub(super) tracker: TaskTracker,
}

fn delivery_error<SenderError>(error: &OpenMeterError<SenderError>) -> LoggerError
where
    SenderError: Error + 'static,
{
    LoggerError::Delivery {
        callback: "openmeter".to_owned(),
        message: error.to_string(),
    }
}

// LiteLLM's sync hooks are sync and post on the blocking client, so they go onto the owned tracker
#[async_trait]
impl<Sender: MeteringSender + 'static, Time: UtcClock + 'static> CallbackLogger
    for OpenMeterAdapter<Sender, Time>
{
    fn callback_name(&self) -> &'static str {
        "openmeter"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = metering_event(&payload);
            if let Err(error) = logger.log_success_event(&call).await {
                tracing::warn!(%error, "the openmeter success event could not be logged");
            }
        });
        Ok(())
    }

    fn log_stream_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = metering_event(&payload);
            if let Err(error) = logger.log_success_event(&call).await {
                tracing::warn!(%error, "the openmeter stream event could not be logged");
            }
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let call = metering_event(&event.payload);
        self.logger
            .async_log_success_event(&call)
            .await
            .map_err(|error| delivery_error(&error))
    }

    async fn async_log_stream_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let call = metering_event(&event.payload);
        self.logger
            .async_log_success_event(&call)
            .await
            .map_err(|error| delivery_error(&error))
    }
}
