use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::anthropic_capability_catalog::capability::supports_model_capability;

// The levels _model_supports_effort_param probes, in LiteLLM's order.
const EFFORT_LEVELS: [&str; 6] = ["low", "minimal", "medium", "high", "xhigh", "max"];

// supports_output_config, or any level flag, means the model accepts output_config.effort at all.
pub async fn model_supports_effort_param<Source, Transport>(
    model: &str,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if supports_model_capability(
        model,
        "supports_output_config",
        custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        return true;
    }
    for level in EFFORT_LEVELS {
        let key = format!("supports_{level}_reasoning_effort");
        if supports_model_capability(model, &key, custom_llm_provider, environment, catalogs).await
        {
            return true;
        }
    }
    false
}
