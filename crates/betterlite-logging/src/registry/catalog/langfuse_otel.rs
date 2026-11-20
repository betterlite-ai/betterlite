use std::sync::Arc;

use super::super::{CatalogError, CatalogInputs};
use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use crate::callback_logger::CallbackLogger;
use crate::langfuse_otel::LangfuseOtelLogger;
use crate::otel::legacy_lifecycle::lifecycle::LegacyLifecycle;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel_provider_assembly::legacy_otel_init::{LegacyOtelEnvironment, LegacyOtelInit};

pub const CALLBACK_NAME: &str = "langfuse_otel";

// litellm_logging's langfuse_otel arm: V2 first, then B3910's logger, which builds its own LANGFUSE_* config
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let init = LegacyOtelInit {
        callback_name: Some(CALLBACK_NAME.to_owned()),
        ..LegacyOtelInit::default()
    };
    let environment = LegacyOtelEnvironment {
        env: &shared.env,
        clients: shared.clients(inputs)?,
        shared_exporter: shared.shared_exporter.clone(),
        globals: &shared.globals,
        proxy: Some(&shared.proxy),
    };
    let logger =
        LangfuseOtelLogger::new(init, &environment).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    let mut prepared = legacy::complete(inputs, shared, logger.otel)?;
    let options = prepared.options()?;
    let lifecycle = LegacyLifecycle::from_otel(prepared.otel, options);
    Ok(legacy::lifecycle_logger(lifecycle))
}

mod tests;
