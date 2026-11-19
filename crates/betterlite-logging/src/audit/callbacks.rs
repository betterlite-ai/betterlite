use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::settings::JsonValue;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::s3_audit::dedicated::build_audit_logger;
use crate::s3_v2::source::Source;

const DEDICATED_S3_CALLBACK: &str = "s3_v2";

#[derive(Debug, thiserror::Error)]
pub enum AuditLoggerBuildError {
    #[error("the s3 audit callback parameters are not valid")]
    Parameters {
        #[source]
        source: serde_json::Error,
    },
    #[error("the dedicated s3 audit logger could not be built")]
    Build {
        #[source]
        source: CatalogError,
    },
}

// LiteLLM's S3Logger(s3_callback_params_override=litellm.s3_audit_callback_params), injected so this module never owns B115
pub trait DedicatedAuditLoggerBuilder: Send + Sync {
    fn build(&self, params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError>;
}

pub struct CatalogAuditLoggerBuilder {
    pub inputs: CatalogInputs,
}

impl DedicatedAuditLoggerBuilder for CatalogAuditLoggerBuilder {
    fn build(&self, params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        let source: Source<()> = serde_json::from_value(params.clone())
            .map_err(|error| AuditLoggerBuildError::Parameters { source: error })?;
        let logger = build_audit_logger(&self.inputs, &source)
            .map_err(|error| AuditLoggerBuildError::Build { source: error })?;
        Ok(logger)
    }
}

// LiteLLM's _audit_log_callback_cache, keyed by the configured callback name
pub struct NamedAuditCallbacks {
    pub resolver: Arc<dyn LoggerResolver>,
    pub dedicated: Arc<dyn DedicatedAuditLoggerBuilder>,
    pub cache: Mutex<BTreeMap<String, Arc<dyn CallbackLogger>>>,
}

impl NamedAuditCallbacks {
    pub fn reset_audit_log_callback_cache(&self) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    // a name resolves once, and only a resolved logger enters the cache, so a failed build retries on the next event
    pub fn resolve(
        &self,
        name: &str,
        s3_params: Option<&JsonValue>,
    ) -> Option<Arc<dyn CallbackLogger>> {
        if let Some(cached) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
        {
            return Some(cached);
        }
        let resolved = match (name, s3_params) {
            (DEDICATED_S3_CALLBACK, Some(params)) => match self.dedicated.build(params) {
                Ok(logger) => logger,
                Err(error) => {
                    tracing::error!("Failed dispatching audit log to callback: {error}");
                    return None;
                }
            },
            _ => match self.resolver.resolve(name, &BTreeMap::new()) {
                Ok(Some(logger)) => logger,
                Ok(None) => {
                    tracing::warn!("Could not resolve audit log callback: {name}");
                    return None;
                }
                Err(error) => {
                    tracing::warn!(%error, "Could not resolve audit log callback: {name}");
                    return None;
                }
            },
        };
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), Arc::clone(&resolved));
        Some(resolved)
    }
}
