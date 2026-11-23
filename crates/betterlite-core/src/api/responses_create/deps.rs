use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::file_ids::mapping::FileIdMapping;

use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::service::SuppliedClient;
use crate::api::responses_create_http::config::CreateConfig;
use crate::api::responses_create_http::service::{ClientSource, CreateHooks};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::prompting::PromptManager;
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::ModelResolver;

use super::bridge::ChatBridge;
use super::error::CreateParamError;
use super::interception::CreateInterception;

// What the factory needs to build a provider's config: the resolved model and provider, the caller's base and the deployment's model info
pub struct ConfigRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub api_base: Option<&'request str>,
    pub model_info: &'request JsonValue,
}

// Two provider capabilities the create wrapper reads before the config is asked for a request
pub trait CreateCapabilities {
    fn supports_native_file_search(&self) -> bool;

    fn should_fake_stream(
        &self,
        model: &str,
        stream: Option<bool>,
        custom_llm_provider: &str,
    ) -> bool;
}

// B157 decides whether a provider has a responses config; the factory builds the object every later step reads
pub trait CreateConfigFactory: Send + Sync {
    type Provider: CreateConfig
        + ResponsesParamProvider<Error = CreateParamError>
        + CreateCapabilities;

    fn create_config(&self, request: &ConfigRequest<'_>) -> Option<Self::Provider>;
}

pub struct ResponsesCreateDeps<'deps, S, F, D: ?Sized, E: ?Sized, Context: Send>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
{
    pub resolver: &'deps (dyn ModelResolver + Send + Sync),
    pub lookup: &'deps (dyn ConfigLookup + Send + Sync),
    pub configs: &'deps F,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub environment: &'deps dyn EnvironmentLookup,
    pub prompts: Option<&'deps dyn PromptManager>,
    pub file_mapping: Option<&'deps FileIdMapping>,
    pub hooks: CreateHooks<'deps, D, E>,
    pub interception: &'deps dyn CreateInterception<Context>,
    pub bridge: &'deps dyn ChatBridge<Context>,
    pub drop_params: bool,
    pub reasoning_auto_summary: bool,
    // litellm.mock_response, which a call's own mock_response kwarg overrides
    pub mock_response: Option<String>,
}
