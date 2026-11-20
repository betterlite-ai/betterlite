pub mod gate;
pub mod hook_call;
pub mod selection;

mod json_bridge;
mod tests;

use std::error::Error;
use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPromptRequest, CompletionFuture, PromptManagementBackend,
};
use thiserror::Error;

use crate::api::vector_store_registry::store::VectorStoreRegistry;

// LiteLLM's CustomPromptManagement: one object-safe trait so a single list holds every backend.
pub trait PromptManager<BackendError>: Sync
where
    BackendError: Error + Send + Sync + 'static,
{
    fn integration_name(&self) -> &'static str;

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> Result<bool, PromptCheckError>;

    fn get_chat_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, BackendError>;

    // LiteLLM's async_get_chat_completion_prompt; the 30-character name rule shortens it.
    fn async_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, BackendError>;
}

// B4058's associated error type keeps its trait from being object safe, so the object-safe trait delegates to it.
impl<Backend> PromptManager<<Backend as PromptManagementBackend>::CompileError> for Backend
where
    Backend: PromptManagementBackend + Send + Sync + 'static,
{
    fn integration_name(&self) -> &'static str {
        PromptManagementBackend::integration_name(self)
    }

    fn should_run(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> Result<bool, PromptCheckError> {
        Ok(PromptManagementBackend::should_run(
            self,
            prompt_id,
            prompt_spec,
            dynamic_callback_params,
        ))
    }

    fn get_chat_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, Backend::CompileError> {
        PromptManagementBackend::get_chat_completion_prompt(self, request)
    }

    fn async_completion_prompt<'request>(
        &'request self,
        request: ChatCompletionPromptRequest<'request>,
    ) -> CompletionFuture<'request, Backend::CompileError> {
        PromptManagementBackend::async_completion_prompt(self, request)
    }
}

// LiteLLM's bare except around should_run_prompt_management; a check nobody can make counts as no.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("the prompt manager could not tell whether it runs")]
pub struct PromptCheckError;

// _init_custom_logger_compatible_class without the usage cache or the router; the server builds the real one.
pub trait CompatibleLoggerFactory<BackendError>: Sync
where
    BackendError: Error + Send + Sync + 'static,
{
    fn known_callbacks(&self) -> &[&'static str];

    fn init(&self, name: &str) -> Option<Arc<dyn PromptManager<BackendError> + Send + Sync>>;
}

pub struct PromptHookDispatch<'call, BackendError>
where
    BackendError: Error + Send + Sync + 'static,
{
    state: &'call mut LoggingState,
    managers: &'call mut Vec<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
    callbacks: &'call mut Vec<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
    factory: &'call dyn CompatibleLoggerFactory<BackendError>,
    vector_stores: Option<&'call VectorStoreRegistry>,
}

impl<'call, BackendError> PromptHookDispatch<'call, BackendError>
where
    BackendError: Error + Send + Sync + 'static,
{
    // managers holds the prompt-management loggers selection walks; callbacks is B189's global list the vector store hook joins.
    #[must_use]
    pub const fn new(
        state: &'call mut LoggingState,
        managers: &'call mut Vec<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
        callbacks: &'call mut Vec<Arc<dyn PromptManager<BackendError> + Send + Sync>>,
        factory: &'call dyn CompatibleLoggerFactory<BackendError>,
        vector_stores: Option<&'call VectorStoreRegistry>,
    ) -> Self {
        Self {
            state,
            managers,
            callbacks,
            factory,
            vector_stores,
        }
    }
}
