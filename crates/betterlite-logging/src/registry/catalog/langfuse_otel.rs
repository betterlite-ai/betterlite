use std::sync::Arc;

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::langfuse_otel::LangfuseOtelLogger;
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
};

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
        config: None,
        callback_name: Some(CALLBACK_NAME.to_owned()),
        overrides: MetadataKeyOverrides::default(),
        injected: InjectedProviders::default(),
        max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
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
    legacy::complete(inputs, shared, logger.otel)
}

mod tests;
