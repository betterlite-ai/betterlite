use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use super::payload::{callback_kwargs, utc_at};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::athina::AthinaLogger;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::http_analytics_loggers::event::Event;

pub(super) struct AthinaAdapter<D: Delivery> {
    pub(super) logger: Arc<AthinaLogger<D>>,
    pub(super) tracker: TaskTracker,
}

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "athina".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}

// LiteLLM runs the athina logger inside the sync success handler, which also serves async calls, so betterlite starts the same delivery from both hooks
#[async_trait]
impl<D: Delivery + 'static> CallbackLogger for AthinaAdapter<D> {
    fn callback_name(&self) -> &'static str {
        "athina"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let start_time = utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?;
        let end_time = utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?;
        let kwargs = callback_kwargs(&payload, event.stream);
        let response = payload.response.clone();
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = Event {
                kwargs: &kwargs,
                response: response.as_ref(),
                start_time,
                end_time,
            };
            logger.log_event(&call).await;
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_success_event(&event)
    }
}
