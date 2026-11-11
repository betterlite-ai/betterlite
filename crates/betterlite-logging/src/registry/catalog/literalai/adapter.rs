use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio_util::task::TaskTracker;

use super::payload::{callback_kwargs, utc_at};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::literal_ai::literal_ai_logging::ids::IdSource;
use crate::literal_ai::literal_ai_logging::payload::CallbackKwargs;
use crate::literal_ai::literal_ai_logging::{Call, LiteralAILogger};

pub(super) struct LiteralAiAdapter<D, I> {
    pub(super) logger: Arc<LiteralAILogger<D, I>>,
    pub(super) tracker: TaskTracker,
}

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "literalai".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}

fn prepare(
    event: &LoggerEvent,
) -> Result<(CallbackKwargs, DateTime<Utc>, DateTime<Utc>), LoggerError> {
    let payload = &event.payload;
    let start_time = utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?;
    let end_time = utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?;
    Ok((callback_kwargs(payload), start_time, end_time))
}

// LiteLLM's batch logger queues the step and sends when the batch size is reached, so the sync hook runs the same async body
#[async_trait]
impl<D: Delivery + 'static, I: IdSource + 'static> CallbackLogger for LiteralAiAdapter<D, I> {
    fn callback_name(&self) -> &'static str {
        "literalai"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let (kwargs, start_time, end_time) = prepare(event)?;
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = Call {
                kwargs: &kwargs,
                start_time,
                end_time,
            };
            logger.log_success_event(call).await;
        });
        Ok(())
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let (kwargs, start_time, end_time) = prepare(event)?;
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = Call {
                kwargs: &kwargs,
                start_time,
                end_time,
            };
            logger.log_failure_event(call).await;
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let (kwargs, start_time, end_time) = prepare(&event)?;
        let call = Call {
            kwargs: &kwargs,
            start_time,
            end_time,
        };
        self.logger.async_log_success_event(call).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let (kwargs, start_time, end_time) = prepare(&event)?;
        let call = Call {
            kwargs: &kwargs,
            start_time,
            end_time,
        };
        self.logger.async_log_failure_event(call).await;
        Ok(())
    }
}
