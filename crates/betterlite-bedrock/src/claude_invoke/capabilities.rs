use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::invoke_schema::InvokeSchemaCapabilities;

use super::CUSTOM_LLM_PROVIDER;

// B597's schema helpers read both capability answers through the Bedrock provider.
pub(super) struct InvokeCapabilities<'context, 'environment, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub(super) environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub(super) catalogs: &'context CatalogSources<'environment>,
}

impl<Source, Transport> InvokeSchemaCapabilities for InvokeCapabilities<'_, '_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    async fn supports_factory(&mut self, request: &CapabilityRequest<'_>, key: &str) -> bool {
        self.environment.supports_factory(request, key).await
    }

    async fn supports_effort_param(&mut self, model: &str) -> bool {
        model_supports_effort_param(
            model,
            CUSTOM_LLM_PROVIDER,
            &mut *self.environment,
            self.catalogs,
        )
        .await
    }
}
