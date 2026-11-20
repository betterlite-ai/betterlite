use std::sync::Arc;

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::legacy_policy::config::{Exporter, OTLP_HTTP_EXPORTER, OpenTelemetryConfig};
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::presets::signoz::SIGNOZ_INGESTION_ENDPOINT_ENV;

pub const CALLBACK_NAME: &str = "signoz";
const INGESTION_KEY_VARIABLE: &str = "SIGNOZ_INGESTION_KEY";
const TRACES_PATH: &str = "/v1/traces";

// litellm_logging's signoz arm: the ingestion endpoint first, then V2, then the endpoint and key it computes
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

// the ingestion endpoint with /v1/traces appended, and the key's header when one is set
pub(crate) fn legacy_config(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
) -> Result<OpenTelemetryConfig, CatalogError> {
    let base = inputs
        .env
        .lookup(SIGNOZ_INGESTION_ENDPOINT_ENV)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CatalogError::Init {
            message: format!("{SIGNOZ_INGESTION_ENDPOINT_ENV} not found in environment variables"),
        })?;
    let base = base.trim_end_matches('/');
    let endpoint = if base.ends_with(TRACES_PATH) {
        base.to_owned()
    } else {
        format!("{base}{TRACES_PATH}")
    };
    let headers = inputs
        .env
        .lookup(INGESTION_KEY_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(|key| format!("signoz-ingestion-key={key}"));
    Ok(super::otel::vendor_config(
        Exporter::Named(OTLP_HTTP_EXPORTER.to_owned()),
        endpoint,
        headers,
        shared,
    ))
}

mod tests;
