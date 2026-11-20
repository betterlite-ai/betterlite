use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup as _;

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::legacy_policy::config::{Exporter, OTLP_HTTP_EXPORTER, OpenTelemetryConfig};
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "logfire";
const TOKEN_VARIABLE: &str = "LOGFIRE_TOKEN";
const BASE_URL_VARIABLE: &str = "LOGFIRE_BASE_URL";
const DEFAULT_BASE_URL: &str = "https://logfire-api.pydantic.dev";
const TRACES_PATH: &str = "/v1/traces";
const MISSING_TOKEN: &str = "LOGFIRE_TOKEN not found in environment variables";

// litellm_logging's logfire arm: the token first, then the plain legacy OpenTelemetry over the logfire URL
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
    // LiteLLM constructs OpenTelemetry(config=otel_config) with no callback name
    let prepared = legacy::prepare(inputs, shared, config, None)?;
    legacy::plain(prepared)
}

// the logfire traces URL, its default base, and the bearer header the token becomes
pub(crate) fn legacy_config(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
) -> Result<OpenTelemetryConfig, CatalogError> {
    let token = inputs
        .env
        .lookup(TOKEN_VARIABLE)
        .ok_or_else(|| CatalogError::Init {
            message: MISSING_TOKEN.to_owned(),
        })?;
    let base = inputs
        .env
        .lookup(BASE_URL_VARIABLE)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let endpoint = format!("{}{TRACES_PATH}", base.trim_end_matches('/'));
    Ok(super::otel::vendor_config(
        Exporter::Named(OTLP_HTTP_EXPORTER.to_owned()),
        endpoint,
        Some(format!("Authorization={token}")),
        shared,
    ))
}

mod tests;
