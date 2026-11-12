use std::future::Future;

use crate::decisions::Json;
use crate::fireworks_catalog_metadata::Catalog;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::MetadataEnvironment;
use crate::model_metadata_resolution::hf_config::HfConfigClient;

mod details;
mod predicates;
mod raw;
mod tests;

pub struct CapabilityRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
}

// Everything B2219's metadata lookup, B2217's bare key and B2220's provider info read, passed in by the caller.
pub struct CapabilityEnvironment<'environment, Source, Transport> {
    pub metadata: MetadataEnvironment<'environment>,
    pub keys: &'environment mut ModelCostKeys,
    pub dynamic: &'environment Source,
    pub hf: &'environment HfConfigClient<Transport>,
    pub provider_catalog: &'environment Catalog,
    pub provider_generation: u64,
}

// LiteLLM's nine named capability delegates, each one shared-factory call with the key its name spells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamedCapability {
    SystemMessages,
    ParallelCalls,
    FunctionCalling,
    ToolChoice,
    PromptCaching,
    Reasoning,
    NoneReasoningEffort,
    MidConversationSystem,
    NativeStructuredOutput,
}

impl NamedCapability {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::SystemMessages => "supports_system_messages",
            Self::ParallelCalls => "supports_parallel_function_calling",
            Self::FunctionCalling => "supports_function_calling",
            Self::ToolChoice => "supports_tool_choice",
            Self::PromptCaching => "supports_prompt_caching",
            Self::Reasoning => "supports_reasoning",
            Self::NoneReasoningEffort => "supports_none_reasoning_effort",
            Self::MidConversationSystem => "supports_mid_conversation_system",
            Self::NativeStructuredOutput => "supports_native_structured_output",
        }
    }
}

pub trait CapabilityPredicates {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send;

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send;

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send;

    fn supports_native_streaming(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send;

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send;
}
