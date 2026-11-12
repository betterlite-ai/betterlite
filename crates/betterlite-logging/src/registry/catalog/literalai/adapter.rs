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

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Failure,
}

struct Prepared {
    kwargs: CallbackKwargs,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
}

impl Prepared {
    fn new(event: &LoggerEvent) -> Result<Self, LoggerError> {
        let payload = &event.payload;
        Ok(Self {
            kwargs: callback_kwargs(payload),
            start_time: utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?,
            end_time: utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?,
        })
    }

    const fn call(&self) -> Call<'_> {
        Call {
            kwargs: &self.kwargs,
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }
}

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "literalai".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}

impl<D: Delivery + 'static, I: IdSource + 'static> LiteralAiAdapter<D, I> {
    fn started(&self, event: &LoggerEvent, outcome: Outcome) -> Result<(), LoggerError> {
        let prepared = Prepared::new(event)?;
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = prepared.call();
            match outcome {
                Outcome::Success => logger.log_success_event(call).await,
                Outcome::Failure => logger.log_failure_event(call).await,
            }
        });
        Ok(())
    }

    async fn finished(&self, event: &LoggerEvent, outcome: Outcome) -> Result<(), LoggerError> {
        let prepared = Prepared::new(event)?;
        match outcome {
            Outcome::Success => self.logger.async_log_success_event(prepared.call()).await,
            Outcome::Failure => self.logger.async_log_failure_event(prepared.call()).await,
        }
        Ok(())
    }
}

// LiteLLM's batch logger queues the step and sends when the batch size is reached, so the sync hook runs the same async body
#[async_trait]
impl<D: Delivery + 'static, I: IdSource + 'static> CallbackLogger for LiteralAiAdapter<D, I> {
    fn callback_name(&self) -> &'static str {
        "literalai"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.started(event, Outcome::Success)
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.started(event, Outcome::Failure)
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.finished(&event, Outcome::Success).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.finished(&event, Outcome::Failure).await
    }
}
