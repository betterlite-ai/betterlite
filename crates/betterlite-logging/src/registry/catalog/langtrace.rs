use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup as _;

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::integrations::langtrace::langtrace_trace_endpoint;
use crate::otel::legacy_policy::config::{Exporter, OTLP_HTTP_EXPORTER, OpenTelemetryConfig};
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "langtrace";
const API_KEY_VARIABLE: &str = "LANGTRACE_API_KEY";
const MISSING_KEY: &str = "LANGTRACE_API_KEY not found in environment variables";

// litellm_logging's langtrace arm: the required key first, then V2, then the legacy config it computes
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let config = legacy_config(inputs, shared)?;
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let prepared = legacy::prepare(inputs, shared, config, Some(CALLBACK_NAME))?;
    legacy::plain(prepared)
}

// the endpoint langtrace_trace_endpoint computes from LANGTRACE_API_HOST and the key's header
pub(crate) fn legacy_config(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
) -> Result<OpenTelemetryConfig, CatalogError> {
    let api_key = inputs
        .env
        .lookup(API_KEY_VARIABLE)
        .ok_or_else(|| CatalogError::Init {
            message: MISSING_KEY.to_owned(),
        })?;
    let endpoint = langtrace_trace_endpoint(inputs.env.lookup("LANGTRACE_API_HOST").as_deref());
    Ok(super::otel::vendor_config(
        Exporter::Named(OTLP_HTTP_EXPORTER.to_owned()),
        endpoint,
        Some(format!("x-api-key={api_key}")),
        shared,
    ))
}

mod tests;
