use super::args::JsonObject;
use super::error::PricingFailure;
use async_trait::async_trait;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_core::responses_bridge::ResponsesBridgeCapabilities;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::decisions::Json;
use betterlite_models::params::{
    ChatParameters, ParameterError, ParameterMapping, ParameterPolicy,
};
use betterlite_models::request_pricing::register::CustomPricingRequest;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use thiserror::Error;
pub struct ProviderLookup<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
}
pub struct ChatPolicy<'config> {
    pub translates_developer_role: bool,
    pub mapper: Option<&'config dyn OptionalParamsMapper>,
}
pub struct ParameterMappingRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub base_model: Option<&'request str>,
    pub parameters: &'request ChatParameters,
    pub policy: &'request ParameterPolicy,
}
pub struct CacheSeedRequest<'request> {
    pub messages: &'request [Message],
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub tools: Option<&'request [Json]>,
    pub enable_prompt_caching: Option<bool>,
    pub api_base: Option<&'request str>,
}
pub struct PromptRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub non_default_params: JsonObject,
    pub prompt_id: Option<String>,
    pub prompt_variables: Option<JsonObject>,
    pub prompt_label: Option<String>,
    pub prompt_version: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct PromptOutcome {
    pub model: String,
    pub messages: Vec<Message>,
    pub non_default_params: JsonObject,
}
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PromptManagementError {
    pub status_code: u16,
    pub message: String,
}
pub struct EnvironmentUpdate<'update> {
    pub model: &'update str,
    pub user: Option<&'update str>,
    pub optional_params: &'update JsonObject,
    pub litellm_params: &'update CallParams,
    pub custom_llm_provider: Option<&'update str>,
}
pub trait ModelMetadata: Sync {
    fn resolve_provider(
        &self,
        request: &ProviderLookup<'_>,
    ) -> Result<ResolvedModel, ResolutionError>;
    fn chat_policy<'config>(
        &'config self,
        model: &str,
        provider: &str,
        base_model: Option<&str>,
    ) -> ChatPolicy<'config>;
    fn parameter_mapping(
        &self,
        request: &ParameterMappingRequest<'_>,
    ) -> Result<ParameterMapping, ParameterError>;
}
#[async_trait]
pub trait PromptManagement: Sync {
    fn seed_cache_control(
        &self,
        request: &CacheSeedRequest<'_>,
        non_default_params: &mut JsonObject,
    );
    fn should_run(
        &self,
        prompt_id: Option<&str>,
        non_default_params: &JsonObject,
        tools: Option<&[Json]>,
    ) -> bool;
    async fn get_chat_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError>;
    async fn async_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError>;
}
#[async_trait]
pub trait PricingRegistration: Sync {
    async fn register_custom_pricing(
        &self,
        request: &CustomPricingRequest<'_>,
    ) -> Result<(), PricingFailure>;
}
pub trait CompletionLogging: Sync {
    fn update_environment(&self, update: &EnvironmentUpdate<'_>);
}
pub struct CompletionPorts<'ports> {
    pub model_metadata: &'ports dyn ModelMetadata,
    pub prompt: &'ports dyn PromptManagement,
    pub bridge: &'ports (dyn ResponsesBridgeCapabilities + Sync),
    pub pricing: &'ports dyn PricingRegistration,
    pub logging: &'ports dyn CompletionLogging,
}
