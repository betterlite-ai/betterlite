use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

const TOOL_SEARCH_CAPABILITY: &str = "supports_tool_search";

#[must_use]
pub async fn supports_tool_search<Source, Transport>(
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    supports_model_capability(
        model,
        TOOL_SEARCH_CAPABILITY,
        "bedrock",
        environment,
        catalogs,
    )
    .await
}
