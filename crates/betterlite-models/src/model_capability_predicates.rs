use std::future::Future;

use crate::decisions::Json;
use crate::fireworks_catalog_metadata::Catalog;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::MetadataEnvironment;
use crate::model_metadata_resolution::hf_config::HfConfigClient;

mod details;
mod predicates;
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

// The nine public delegates are default methods so each stays a named row without a one-line free function.
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

    fn supports_system_messages(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_system_messages")
    }

    fn supports_parallel_function_calling(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_parallel_function_calling")
    }

    fn supports_function_calling(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_function_calling")
    }

    fn supports_tool_choice(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_tool_choice")
    }

    fn supports_prompt_caching(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_prompt_caching")
    }

    fn supports_reasoning(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_reasoning")
    }

    fn supports_none_reasoning_effort(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_none_reasoning_effort")
    }

    fn supports_mid_conversation_system(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_mid_conversation_system")
    }

    fn supports_native_structured_output(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.supports_factory(request, "supports_native_structured_output")
    }
}
