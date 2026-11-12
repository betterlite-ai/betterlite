use std::error::Error as StdError;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio_util::task::TaskTracker;

use super::payload::callback_kwargs;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::deepeval::deepeval_logging::ids::UuidSource;
use crate::deepeval::deepeval_logging::payload::CallbackKwargs;
use crate::deepeval::deepeval_logging::{Call, DeepEvalError, DeepEvalLogger};
use crate::http_analytics_loggers::delivery::Delivery;

pub(super) struct DeepevalAdapter<D, I> {
    pub(super) logger: Arc<DeepEvalLogger<D, I>>,
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
        callback: "deepeval".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
}

fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
}

fn delivery_error<E>(error: &DeepEvalError<E>) -> LoggerError
where
    E: StdError + 'static,
{
    LoggerError::Delivery {
        callback: "deepeval".to_owned(),
        message: error.to_string(),
    }
}

impl<D: Delivery + 'static, I: UuidSource + 'static> DeepevalAdapter<D, I> {
    fn started(&self, prepared: Prepared, outcome: Outcome) {
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            let call = prepared.call();
            let answer = match outcome {
                Outcome::Success => logger.log_success_event(call).await,
                Outcome::Failure => logger.log_failure_event(call).await,
            };
            if let Err(error) = answer {
                let mapped = delivery_error(&error);
                tracing::warn!(%mapped, "the deepeval event could not be logged");
            }
        });
    }

    async fn finished(&self, prepared: &Prepared, outcome: Outcome) -> Result<(), LoggerError> {
        match outcome {
            Outcome::Success => self.logger.async_log_success_event(prepared.call()).await,
            Outcome::Failure => self.logger.async_log_failure_event(prepared.call()).await,
        }
        .map_err(|error| delivery_error(&error))
    }
}

// LiteLLM runs this logger from both the sync and the async success handler, so each hook reaches its own merged method
#[async_trait]
impl<D: Delivery + 'static, I: UuidSource + 'static> CallbackLogger for DeepevalAdapter<D, I> {
    fn callback_name(&self) -> &'static str {
        "deepeval"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.started(Prepared::new(event)?, Outcome::Success);
        Ok(())
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.started(Prepared::new(event)?, Outcome::Failure);
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let prepared = Prepared::new(&event)?;
        self.finished(&prepared, Outcome::Success).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let prepared = Prepared::new(&event)?;
        self.finished(&prepared, Outcome::Failure).await
    }
}
