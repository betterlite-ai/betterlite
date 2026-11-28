use super::bridge::ChatBridge;
use super::interception::CreateInterception;
use crate::api::responses_compact_http::service::{ClientSource, SuppliedClient};
use crate::api::responses_create_http::config::CreateConfig;
use crate::api::responses_create_http::service::CreateHooks;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::prompting::PromptManager;
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
pub struct ConfigRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub api_base: Option<&'request str>,
    pub model_info: &'request JsonValue,
}
pub trait CreateCapabilities {
    fn supports_native_file_search(&self) -> bool;
    fn should_fake_stream(
        &self,
        model: &str,
        stream: Option<bool>,
        custom_llm_provider: &str,
    ) -> bool;
}
pub trait CreateConfigFactory: Send + Sync {
    type Provider: CreateConfig
        + ResponsesParamProvider<Error = ResponsesParametersError>
        + CreateCapabilities;
    fn create_config(&self, request: &ConfigRequest<'_>) -> Option<Self::Provider>;
}
// The prompt manager enters behind a Send + Sync port so the create future stays Send for the gateway
pub trait PromptSource: PromptManager + Send + Sync {}
pub struct ResponsesCreateDeps<'deps, S: ClientSource, F, D: ?Sized, E: ?Sized, Context: Send> {
    pub resolver: &'deps (dyn ModelResolver + Send + Sync),
    pub lookup: &'deps (dyn ConfigLookup + Send + Sync),
    pub configs: &'deps F,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub environment: &'deps dyn EnvironmentLookup,
    pub prompts: Option<&'deps dyn PromptSource>,
    pub file_mapping: Option<&'deps FileIdMapping>,
    pub hooks: CreateHooks<'deps, D, E>,
    pub interception: &'deps dyn CreateInterception<Context>,
    pub bridge: &'deps dyn ChatBridge<Context>,
    pub drop_params: bool,
    pub reasoning_auto_summary: bool,
}
