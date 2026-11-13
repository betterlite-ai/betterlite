use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest, NamedCapability,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::resolution::{ResolutionRequest, resolve};

pub const CACHE_CONTROL_PROVIDERS: [&str; 4] = ["anthropic", "bedrock", "vertex_ai", "azure_ai"];

#[must_use]
pub async fn supports_cache_control<Source, Transport>(
    model: &str,
    custom_llm_provider: Option<&str>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(provider) = resolved_provider(model, custom_llm_provider, environment) else {
        return false;
    };
    if !CACHE_CONTROL_PROVIDERS.contains(&provider.as_str()) {
        return false;
    }
    if !model.to_lowercase().contains("claude") {
        return false;
    }
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some(&provider),
    };
    environment
        .supports_factory(&request, NamedCapability::PromptCaching.key())
        .await
}

fn resolved_provider<Source, Transport>(
    model: &str,
    custom_llm_provider: Option<&str>,
    environment: &CapabilityEnvironment<'_, Source, Transport>,
) -> Option<String>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if let Some(provider) = custom_llm_provider {
        return Some(provider.to_owned());
    }
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&request, environment.metadata.resolution).ok()?;
    Some(resolved.provider.slug().to_owned())
}
