use std::sync::Arc;

use super::super::{CatalogError, CatalogInputs};
use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::phoenix_callback::{otel_config, phoenix_lifecycle};
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "arize_phoenix";

// litellm_logging's arize_phoenix arm: V2 first, then B3862's ArizePhoenixLogger over its config
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let config = otel_config(&shared.env).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    let mut prepared = legacy::prepare(inputs, shared, config, Some(CALLBACK_NAME))?;
    let options = prepared.options()?;
    let logger = phoenix_lifecycle(prepared.otel, options, &shared.env);
    Ok(legacy::lifecycle_logger(logger))
}

mod tests;
