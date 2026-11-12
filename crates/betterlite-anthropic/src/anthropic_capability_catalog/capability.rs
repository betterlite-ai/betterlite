use std::collections::BTreeMap;

use betterlite_context::anthropic_catalog_lookup::model_capability;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::model_metadata_resolution::{MetadataRequest, get_model_info_helper};
use betterlite_models::resolution::{ResolutionRequest, resolve};

use super::CatalogSources;

pub async fn supports_model_capability<Source, Transport>(
    model: &str,
    key: &str,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if let Some(resolved) =
        provider_resolved_capability(model, key, custom_llm_provider, environment).await
    {
        return resolved;
    }
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some(custom_llm_provider),
    };
    if environment.supports_factory(&request, key).await {
        return true;
    }
    model_capability(
        catalogs.patterns,
        catalogs.runtime,
        catalogs.packaged,
        model,
        key,
    ) == Some(true)
}

// B174 resolves the pair, B2219 reads the key, and a failed or non-boolean answer falls through.
async fn provider_resolved_capability<Source, Transport>(
    model: &str,
    key: &str,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Option<bool>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let resolution = ResolutionRequest {
        model,
        custom_llm_provider: Some(custom_llm_provider),
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&resolution, environment.metadata.resolution).ok()?;
    let metadata = MetadataRequest {
        model: &resolved.model,
        custom_llm_provider: Some(resolved.provider.slug()),
        api_base: None,
        api_key: None,
    };
    let info = get_model_info_helper(
        &metadata,
        &environment.metadata,
        environment.keys,
        environment.dynamic,
        environment.hf,
    )
    .await
    .ok()?;
    let fields: BTreeMap<String, Json> = serde_json::from_str(info.get()).ok()?;
    match fields.get(key) {
        Some(Json::Bool(flag)) => Some(*flag),
        Some(_) | None => None,
    }
}
