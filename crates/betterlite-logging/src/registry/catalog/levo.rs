use std::sync::Arc;

use secrecy::ExposeSecret as _;

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::integrations::levo_settings::LevoConfig;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "levo";

// litellm_logging's levo arm: V2 first, then B3863's LevoLogger over the four LEVOAI_* variables
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let prepared = legacy::prepare(
        inputs,
        shared,
        legacy_config(inputs, shared)?,
        Some(CALLBACK_NAME),
    )?;
    legacy::plain(prepared)
}

// the collector URL and the three-variable header LevoConfig builds from the environment
pub(crate) fn legacy_config(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
) -> Result<OpenTelemetryConfig, CatalogError> {
    let levo = LevoConfig::from_env(inputs.env.as_ref()).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    Ok(super::otel::vendor_config(
        Exporter::Named(levo.protocol.exporter_kind().to_owned()),
        levo.endpoint,
        Some(levo.otlp_auth_headers.expose_secret().to_owned()),
        shared,
    ))
}

mod tests;
