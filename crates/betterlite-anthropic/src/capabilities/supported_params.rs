use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest, NamedCapability,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::anthropic_capability_catalog::capability::supports_model_capability;

pub const BASE_SUPPORTED_PARAMS: [&str; 16] = [
    "stream",
    "stop",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "tools",
    "tool_choice",
    "extra_headers",
    "parallel_tool_calls",
    "response_format",
    "user",
    "web_search_options",
    "speed",
    "context_management",
    "cache_control",
];

// Speed stays advertised even when the speed gate later rejects it for this model.
pub async fn get_supported_openai_params<Source, Transport>(
    model: &str,
    custom_llm_provider: Option<&str>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Vec<&'static str>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let resolved_provider = custom_llm_provider.unwrap_or("anthropic");
    let thinking = model.contains("claude-3-7-sonnet")
        || supports_model_capability(
            model,
            "supports_adaptive_thinking",
            resolved_provider,
            environment,
            catalogs,
        )
        .await
        || environment
            .supports_factory(
                &CapabilityRequest {
                    model,
                    custom_llm_provider,
                },
                NamedCapability::Reasoning.key(),
            )
            .await;
    let mut params = BASE_SUPPORTED_PARAMS.to_vec();
    if thinking {
        params.push("thinking");
        params.push("reasoning_effort");
    }
    params
}
