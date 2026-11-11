use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use super::payload::Prepared;
use crate::braintrust_logging::logger::BraintrustLogger;
use crate::braintrust_logging::payload::CallbackPath;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::Delivery;

pub(super) struct BraintrustAdapter<D> {
    pub(super) logger: Arc<BraintrustLogger<D>>,
    pub(super) tracker: TaskTracker,
}

fn delivery_message(message: String) -> LoggerError {
    LoggerError::Delivery {
        callback: "braintrust".to_owned(),
        message,
    }
}

impl<D: Delivery + 'static> BraintrustAdapter<D> {
    fn started(&self, prepared: Prepared) {
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            if let Err(error) = logger
                .log_success_event(&prepared.event(), CallbackPath::Sync)
                .await
            {
                let mapped = delivery_message(error.to_string());
                tracing::warn!(%mapped, "the braintrust success event could not be logged");
            }
        });
    }

    async fn finished(&self, prepared: &Prepared) -> Result<(), LoggerError> {
        self.logger
            .log_success_event(&prepared.event(), CallbackPath::Async)
            .await
            .map_err(|error| delivery_message(error.to_string()))
    }
}

// LiteLLM's sync success handler calls this logger on the blocking client, so the sync hook starts the same async send
#[async_trait]
impl<D: Delivery + 'static> CallbackLogger for BraintrustAdapter<D> {
    fn callback_name(&self) -> &'static str {
        "braintrust"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let prepared =
            Prepared::new(&event.payload).map_err(|error| delivery_message(error.to_string()))?;
        self.started(prepared);
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let prepared =
            Prepared::new(&event.payload).map_err(|error| delivery_message(error.to_string()))?;
        self.finished(&prepared).await
    }
}
