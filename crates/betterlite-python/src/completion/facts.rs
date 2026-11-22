use super::session::SessionCore;
use betterlite_context::non_default_params::names::DEFAULT_CHAT_COMPLETION_PARAMS;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::params::classification::RESERVED_PARAMETER_NAMES;
use betterlite_models::params::{
    CallType, ParameterError, ParameterMapping, map_listed_parameters,
};
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_openai::openai_base_config::Object;
use betterlite_sdk::completion_call::ports::ModelFacts;
use betterlite_sdk::completion_request::ports::{
    ChatPolicy, ModelMetadata, ParameterMappingRequest, ProviderLookup,
};
use std::sync::Arc;
// The SDK asks where a model lives; a plain Python session answers from the environment it was built with.
pub(crate) struct SessionMetadata<'core> {
    pub(crate) core: &'core SessionCore,
    // litellm.api_key and litellm.api_base, which the caller's settings snapshot carries per call
    pub(crate) globals: GlobalCredentials,
}
impl ModelMetadata for SessionMetadata<'_> {
    fn resolve_provider(
        &self,
        request: &ProviderLookup<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        self.core.resolve(request, &self.globals)
    }
    fn chat_policy<'config>(
        &'config self,
        _model: &str,
        _provider: &str,
        _base_model: Option<&str>,
    ) -> ChatPolicy<'config> {
        ChatPolicy {
            translates_developer_role: false,
            mapper: None,
        }
    }
    // LiteLLM's get_optional_params list for a chat call, the same list the SDK's own session uses
    fn parameter_mapping(
        &self,
        request: &ParameterMappingRequest<'_>,
    ) -> Result<ParameterMapping, ParameterError> {
        let supported: Vec<_> = DEFAULT_CHAT_COMPLETION_PARAMS.split_whitespace().collect();
        let mut kept = request.parameters.clone();
        kept.passthrough
            .retain(|name, _value| !RESERVED_PARAMETER_NAMES.contains(&name.as_str()));
        map_listed_parameters(
            &supported,
            request.model,
            CallType::ChatCompletion,
            &kept,
            request.policy,
        )
    }
}
// The responses bridge reads model_info out of the same catalog LiteLLM's model_cost table backs.
pub(crate) struct CatalogFacts {
    pub(crate) catalog: Arc<ModelCatalog>,
}
impl ModelFacts for CatalogFacts {
    fn model_info(&self, model: &str, _custom_llm_provider: &str) -> Option<Object> {
        let metadata = self.catalog.metadata(model)?;
        let mut info = Object::new();
        info.insert(String::from("model"), serde_json::json!(metadata.model));
        info.insert(
            String::from("provider"),
            serde_json::json!(metadata.provider),
        );
        info.insert(String::from("mode"), serde_json::json!(metadata.mode));
        Some(info)
    }
}
