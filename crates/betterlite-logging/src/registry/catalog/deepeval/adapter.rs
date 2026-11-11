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

fn invalid_time(field: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "deepeval".to_owned(),
        message: format!("the standard logging payload {field} is not a valid timestamp"),
    }
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

fn prepare(
    event: &LoggerEvent,
) -> Result<(CallbackKwargs, DateTime<Utc>, DateTime<Utc>), LoggerError> {
    let payload = &event.payload;
    let start_time = utc_at(payload.start_time).ok_or_else(|| invalid_time("startTime"))?;
    let end_time = utc_at(payload.end_time).ok_or_else(|| invalid_time("endTime"))?;
    Ok((callback_kwargs(payload), start_time, end_time))
}

fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
}

// LiteLLM's sync success handler runs this logger on the same path as the async hook, so both reach the merged methods
#[async_trait]
impl<D: Delivery + 'static, I: UuidSource + 'static> CallbackLogger for DeepevalAdapter<D, I> {
    fn callback_name(&self) -> &'static str {
        "deepeval"
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
            if let Err(error) = logger.log_success_event(call).await {
                let mapped = delivery_error(&error);
                tracing::warn!(%mapped, "the deepeval success event could not be logged");
            }
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
            if let Err(error) = logger.log_failure_event(call).await {
                let mapped = delivery_error(&error);
                tracing::warn!(%mapped, "the deepeval failure event could not be logged");
            }
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
        self.logger
            .async_log_success_event(call)
            .await
            .map_err(|error| delivery_error(&error))
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let (kwargs, start_time, end_time) = prepare(&event)?;
        let call = Call {
            kwargs: &kwargs,
            start_time,
            end_time,
        };
        self.logger
            .async_log_failure_event(call)
            .await
            .map_err(|error| delivery_error(&error))
    }
}
