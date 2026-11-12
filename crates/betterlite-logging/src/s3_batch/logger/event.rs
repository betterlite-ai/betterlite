use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::s3_batch::Element;
use crate::s3_batch::logger::{CALLBACK_NAME, S3BatchLogger};
use crate::s3_batch::ndjson_batch_file::COLD_STORAGE_LOGGER;
use crate::s3_v2::events::{Event, create_batch_element};
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::fmt::Display;
use std::sync::{Arc, PoisonError};
fn element_for(
    logger: &S3BatchLogger,
    event: &LoggerEvent,
) -> Result<Option<Element>, LoggerError> {
    let encoded =
        serde_json::to_value(&*event.payload).map_err(|source| delivery_error(&source))?;
    let payload: Json =
        serde_json::from_value(encoded).map_err(|source| delivery_error(&source))?;
    let start_time = naive_utc(event.payload.start_time).ok_or_else(|| LoggerError::Delivery {
        callback: CALLBACK_NAME.to_owned(),
        message: "the standard logging payload startTime is not a valid timestamp".to_owned(),
    })?;
    let mut cache = logger
        .partition_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    create_batch_element(
        &logger.parameters,
        &Event {
            start_time,
            payload: Some(&payload),
            cold_storage_enabled: logger.settings.cold_storage_logger.as_deref()
                == Some(COLD_STORAGE_LOGGER),
        },
        &mut cache,
        logger.environment.as_ref(),
    )
    .map_err(|source| delivery_error(&source))
}
pub(super) fn enqueue_event(
    logger: &S3BatchLogger,
    event: &LoggerEvent,
) -> Result<(), LoggerError> {
    let element = match element_for(logger, event) {
        Ok(element) => element,
        Err(error) => {
            tracing::error!(%error, "s3 Layer Error");
            return Err(error);
        }
    };
    let Some(record) = element else {
        tracing::debug!(
            call_type = %event.payload.call_type,
            "s3 Logging - skipping event, no standard_logging_object"
        );
        return Ok(());
    };
    logger.queue.enqueue(record);
    Ok(())
}
#[async_trait]
impl CallbackLogger for S3BatchLogger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }
    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        enqueue_event(self, &event)
    }
    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.async_log_success_event(event).await
    }
}
fn naive_utc(seconds: Decimal) -> Option<NaiveDateTime> {
    seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|milliseconds| milliseconds.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|moment| moment.naive_utc())
}
fn delivery_error<Source>(source: &Source) -> LoggerError
where
    Source: Display,
{
    LoggerError::Delivery {
        callback: CALLBACK_NAME.to_owned(),
        message: source.to_string(),
    }
}
