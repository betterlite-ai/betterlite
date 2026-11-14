pub(crate) mod bridge;
mod callbacks;
mod clients;
mod compilation;
mod completion;
mod prompt;
mod template;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::service_trace_id_cache::{ServiceTraceIDCache, ServiceTraceIdCacheError};
use thiserror::Error;

use crate::langfuse_logger::langfuse::LangFuseLogger;
use crate::langfuse_logger::{LangfuseLoggerError, LangfuseLoggerRequest, LangfuseServices};
use crate::langfuse_rest::client::LangfuseClientError;
use crate::langfuse_rest::errors::LangfuseRestError;
use crate::langfuse_settings::CredentialRequest;

use self::clients::LangfusePromptClients;

pub const INTEGRATION_NAME: &str = "langfuse";

// LangfusePromptManagement.__init__'s flush_interval default
const PROMPT_MANAGER_FLUSH_INTERVAL: u64 = 1;

// litellm.integrations.langfuse.langfuse_prompt_management.LangfusePromptManagement
pub struct LangfusePromptManagement {
    pub(crate) services: LangfuseServices,
    pub(crate) logger: Arc<LangFuseLogger>,
    pub(crate) clients: LangfusePromptClients,
    pub(crate) trace_cache: Arc<ServiceTraceIDCache>,
}

impl LangfusePromptManagement {
    // LangfusePromptManagement.__init__: one prompt client and one tracing channel for the instance
    pub async fn new(
        services: LangfuseServices,
        trace_cache: Arc<ServiceTraceIDCache>,
    ) -> Result<Self, LangfusePromptManagementError> {
        let logger = LangFuseLogger::new(
            &services,
            &LangfuseLoggerRequest {
                credentials: CredentialRequest {
                    public_key: None,
                    secret: None,
                    secret_key: None,
                    host: None,
                    allow_env_credentials: true,
                },
                environment: None,
                flush_interval: PROMPT_MANAGER_FLUSH_INTERVAL,
            },
        )
        .await?;
        Ok(Self {
            services,
            logger: Arc::new(logger),
            clients: LangfusePromptClients::default(),
            trace_cache,
        })
    }
}

// the wall clock both caches age their entries with
pub(crate) fn unix_seconds() -> f64 {
    let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0.0;
    };
    elapsed.as_secs_f64()
}

#[derive(Debug, Error)]
pub enum LangfusePromptManagementError {
    #[error("prompt_id is required for Langfuse prompt management")]
    MissingPromptId,
    #[error("standard_callback_dynamic_params must be an object")]
    ParamsType,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    FieldType { field: &'static str },
    #[error("the Langfuse prompt config must be an object")]
    ConfigType,
    #[error("the Langfuse prompt config model must be a string")]
    ModelType,
    #[error(transparent)]
    Logger(#[from] LangfuseLoggerError),
    #[error(transparent)]
    Client(#[from] LangfuseClientError),
    #[error(transparent)]
    Rest(#[from] LangfuseRestError),
    #[error(transparent)]
    TraceCache(#[from] ServiceTraceIdCacheError),
}
