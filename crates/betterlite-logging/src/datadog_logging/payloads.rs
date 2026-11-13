use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

use crate::datadog::apm::Tracer;
use crate::datadog::payloads::legacy::{
    LegacyLoggingEvent, LegacyLoggingKwargs, legacy_v1_logging_to_datadog,
};
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::tagging::Environment;
use crate::fields::payload::StandardLoggingPayload;

use super::error::DatadogError;

// LiteLLM's log_success_event kwargs: the standard payload for the default format and the call's own fields for v1
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DatadogLoggingKwargs {
    pub standard_logging_object: Option<StandardLoggingPayload>,
    pub legacy: Option<LegacyLoggingKwargs>,
}

pub struct SyncCall<'event> {
    pub kwargs: &'event DatadogLoggingKwargs,
    pub response_obj: &'event Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

pub fn sync_payload(
    call: &SyncCall<'_>,
    environment: &Environment,
    tracer: &dyn Tracer,
    use_v1: bool,
    generated_id: &str,
) -> Result<DatadogPayload, DatadogError> {
    if use_v1 {
        let Some(kwargs) = call.kwargs.legacy.as_ref() else {
            return Err(DatadogError::MissingLegacyKwargs);
        };
        let event = LegacyLoggingEvent {
            kwargs,
            response_obj: call.response_obj,
            start_time: call.start_time,
            end_time: call.end_time,
            status: DataDogStatus::Info,
            generated_id,
        };
        return legacy_v1_logging_to_datadog(&event, environment, tracer)
            .map_err(DatadogError::from);
    }
    let payload = call
        .kwargs
        .standard_logging_object
        .as_ref()
        .ok_or(DatadogError::MissingStandardLoggingObject)?;
    standard_logging_to_datadog(
        payload,
        DataDogStatus::from_logging_status(payload.status),
        environment,
        tracer,
    )
    .map_err(DatadogError::from)
}
