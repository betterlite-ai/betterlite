use chrono::FixedOffset;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::sync::PoisonError;

use super::delivery::GalileoDelivery;
use super::delivery::buffer::append;
use super::delivery::flush::deliver;
use super::delivery::transport::{TraceIds, Transport};
use crate::galileo_payloads::content::{GalileoKwargs, ResponseObj};
use crate::galileo_payloads::timestamps::RawTimestamp;

mod record;
mod tests;

// litellm.integrations.galileo.GalileoStandardLoggingFields, the fields this callback reads.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StandardLoggingObject {
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    #[serde(default)]
    pub total_tokens: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(
        default,
        rename = "startTime",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub start_time: Option<Decimal>,
    #[serde(
        default,
        rename = "endTime",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub end_time: Option<Decimal>,
}

// The callback kwargs _async_log_success_event_impl reads: the standard logging object and the fields _get_galileo_input_output_content consumes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingObject>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(flatten)]
    pub content: GalileoKwargs,
}

pub struct SuccessEvent<'event> {
    pub kwargs: &'event CallbackKwargs,
    pub response: Option<&'event ResponseObj>,
    pub start_time: Option<RawTimestamp>,
    pub end_time: Option<RawTimestamp>,
    pub local_offset: FixedOffset,
}

#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error("the standard_logging_object time is outside the range a UTC timestamp can hold")]
    Timestamp,
}

// litellm.integrations.galileo.GalileoObserve.async_log_success_event: nothing escapes, an unexpected error is one exception-level line.
pub fn async_log_success_event(
    delivery: &GalileoDelivery,
    transport: &dyn Transport,
    ids: &dyn TraceIds,
    event: &SuccessEvent<'_>,
) {
    tracing::debug!("On Async Success");
    if let Err(error) = log_success_event(delivery, transport, ids, event) {
        tracing::error!("Galileo Logger: unexpected error in async_log_success_event: {error}");
    }
}

// litellm.integrations.galileo.GalileoObserve.async_log_failure_event, diagnostic only: no record is queued and no flush runs.
pub fn async_log_failure_event(kwargs: &CallbackKwargs) {
    tracing::debug!(
        "Galileo Logger: On Async Failure, diagnostic only (call_type={})",
        kwargs.content.call_type.as_deref().unwrap_or("-")
    );
}

// litellm.integrations.galileo.GalileoObserve._async_log_success_event_impl, minus the record construction in record.rs.
fn log_success_event(
    delivery: &GalileoDelivery,
    transport: &dyn Transport,
    ids: &dyn TraceIds,
    event: &SuccessEvent<'_>,
) -> Result<(), AdmissionError> {
    let config = &delivery.config;
    if !config.is_configured() {
        tracing::debug!(
            "Galileo Logger: skipping, GALILEO_PROJECT_ID={} GALILEO_API_KEY={} GALILEO_BASE_URL={}",
            config.project_id.is_some(),
            config.api_key.is_some(),
            config.base_url.is_some(),
        );
        return Ok(());
    }
    let Some(slo) = &event.kwargs.standard_logging_object else {
        tracing::debug!("Galileo Logger: no standard_logging_object in kwargs, skipping");
        return Ok(());
    };
    let Some(record) = record::success_record(event, slo)? else {
        return Ok(());
    };
    append(delivery, record);
    flush_if_due(delivery, transport, ids);
    Ok(())
}

fn flush_if_due(delivery: &GalileoDelivery, transport: &dyn Transport, ids: &dyn TraceIds) {
    let queued = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .len();
    if queued >= delivery.config.batch_size {
        deliver(delivery, transport, ids);
    }
}
