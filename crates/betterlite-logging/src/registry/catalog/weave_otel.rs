use std::sync::Arc;

use secrecy::ExposeSecret as _;

use super::super::{CatalogError, CatalogInputs};
use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::weave::weave_lifecycle;
use crate::otel::integrations::weave_settings::WeaveOtelConfig;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "weave_otel";

// litellm_logging's weave_otel arm: V2 first, then B3863's WeaveOtelLogger over the WANDB_* variables
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let mut prepared = legacy::prepare(
        inputs,
        shared,
        legacy_config(inputs, shared)?,
        Some(CALLBACK_NAME),
    )?;
    let options = prepared.options()?;
    let lifecycle = weave_lifecycle(prepared.otel, options);
    Ok(legacy::lifecycle_logger(lifecycle))
}

// the Weave traces URL and the Authorization,project_id header pair get_weave_otel_config builds
pub(crate) fn legacy_config(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
) -> Result<OpenTelemetryConfig, CatalogError> {
    let weave =
        WeaveOtelConfig::from_env(inputs.env.as_ref()).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    let mut config = OpenTelemetryConfig {
        exporter: Exporter::Named(weave.protocol.exporter_kind().to_owned()),
        endpoint: Some(weave.endpoint),
        headers: Some(weave.otlp_auth_headers.expose_secret().to_owned()),
        ..OpenTelemetryConfig::default()
    };
    config.normalize(&shared.env);
    Ok(config)
}

mod tests;
