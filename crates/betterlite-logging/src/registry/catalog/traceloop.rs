use std::sync::Arc;

use super::super::{CatalogError, CatalogInputs};
use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::traceloop::TraceloopLogger;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "traceloop";

// litellm_logging's set_callbacks arm: Traceloop.init() owns a tracer fed by the OTEL_* variables
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let mut config = OpenTelemetryConfig::from_env(&shared.env);
    config.normalize(&shared.env);
    let prepared = legacy::prepare(inputs, shared, config, Some(CALLBACK_NAME))?;
    // the SDK tracer keeps its provider alive, so the legacy init's other halves can drop with it
    Ok(Arc::new(TraceloopLogger::new(prepared.otel.tracer)))
}

mod tests;
