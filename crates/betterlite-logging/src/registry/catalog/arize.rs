use std::sync::Arc;

use super::super::{CatalogError, CatalogInputs};
use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::arize_callback::{arize_lifecycle, otel_config};
use crate::otel::legacy_lifecycle::logger::LegacyOtelLogger;
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "arize";

// litellm_logging's arize arm: V2 first, then B3862's ArizeLogger over the registration config
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let config = otel_config(&shared.env);
    let mut prepared = legacy::prepare(inputs, shared, config, Some(CALLBACK_NAME))?;
    let options = prepared.options()?;
    let lifecycle = arize_lifecycle(prepared.otel, options);
    Ok(Arc::new(LegacyOtelLogger::new(Arc::new(lifecycle))))
}

mod tests;
