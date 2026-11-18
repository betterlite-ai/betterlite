use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::assembly::assemble_payload;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::tagging::base_tags;
use crate::datadog_logging::logger::DatadogLogger;
use crate::logging_service_events::types::{ServiceLoggerPayload, ServiceType};
use crate::private_json::error::PrivateJsonError;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

// ast-grep-ignore: no-long-names
pub const DD_LOGGED_SUCCESS_SERVICE_TYPES: &[ServiceType] = &[ServiceType::ResetBudgetJob];

#[derive(Debug, Error)]
pub enum ServiceEventError {
    // LiteLLM writes a bare NaN here; a non-finite duration fails the event instead
    #[error("the service duration {0} is not a finite number")]
    Duration(f64),
    #[error("the service payload could not be converted to a JSON object")]
    Payload(#[source] serde_json::Error),
    #[error("the service message could not be encoded")]
    Message(#[from] PrivateJsonError),
}

// ast-grep-ignore: no-one-line-functions
pub fn async_service_failure_hook(
    logger: &DatadogLogger,
    payload: &ServiceLoggerPayload,
    event_metadata: Option<&IndexMap<String, Json>>,
) {
    log_service_event(logger, payload, event_metadata, DataDogStatus::Warn);
}

// litellm.integrations.datadog.datadog.DataDogLogger.async_service_success_hook
pub fn async_service_success_hook(
    logger: &DatadogLogger,
    payload: &ServiceLoggerPayload,
    event_metadata: Option<&IndexMap<String, Json>>,
) {
    // ast-grep-ignore: no-long-names
    if !DD_LOGGED_SUCCESS_SERVICE_TYPES.contains(&payload.service) {
        return;
    }
    log_service_event(logger, payload, event_metadata, DataDogStatus::Info);
}

fn log_service_event(
    logger: &DatadogLogger,
    payload: &ServiceLoggerPayload,
    event_metadata: Option<&IndexMap<String, Json>>,
    status: DataDogStatus,
) {
    if let Err(error) = enqueue_service_event(logger, payload, event_metadata, status) {
        // one text serves both hooks; LiteLLM's success text names the failure hook
        tracing::error!(%error, "Datadog: Logger - Exception in a service hook");
    }
}

fn enqueue_service_event(
    logger: &DatadogLogger,
    payload: &ServiceLoggerPayload,
    event_metadata: Option<&IndexMap<String, Json>>,
    status: DataDogStatus,
) -> Result<(), ServiceEventError> {
    if !payload.duration.is_finite() {
        return Err(ServiceEventError::Duration(payload.duration));
    }
    let mut fields: IndexMap<String, Json> = serde_json::to_value(payload)
        .and_then(serde_json::from_value::<IndexMap<String, Json>>)
        .map_err(ServiceEventError::Payload)?;
    if let Some(metadata) = event_metadata {
        fields.extend(
            metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    let message = safe_dumps(&fields, DEFAULT_MAX_RECURSE_DEPTH, None)?;
    let environment = &logger.settings.environment;
    let dd_payload = assemble_payload(
        environment,
        status,
        &base_tags(environment),
        message,
        // LiteLLM's service hooks never call _add_trace_context_to_payload
        &NullTracer,
    );
    let _enqueued = logger.queue.enqueue(dd_payload);
    Ok(())
}
