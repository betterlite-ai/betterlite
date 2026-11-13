use self::destination::config_from_langfuse_env;
use self::error::LangfuseOtelError;
use crate::otel_provider_assembly::legacy_otel_init::{
    LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, init_legacy_otel,
};

pub mod cloud_endpoints;
pub mod destination;
pub mod dynamic;
pub mod error;
pub mod ingestion_constants;
pub mod service_hooks;

mod tests;

pub struct LangfuseOtelLogger {
    pub otel: LegacyOtel,
}

impl LangfuseOtelLogger {
    // The config is built here and passed down, so the process environment is never rewritten.
    pub fn new(
        mut init: LegacyOtelInit<'_>,
        environment: &LegacyOtelEnvironment<'_>,
    ) -> Result<Self, LangfuseOtelError> {
        if init.config.is_none() {
            init.config = Some(config_from_langfuse_env(environment.env));
        }
        let otel = init_legacy_otel(init, environment)
            .map_err(|source| LangfuseOtelError::Init { source })?;
        Ok(Self { otel })
    }
}
