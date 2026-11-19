use std::convert::Infallible;
use std::error::Error;
use std::future::Future;
use std::sync::PoisonError;

use betterlite_models::full_model_metadata::cache::ModelInfoCache;
use betterlite_models::full_model_metadata::error::FullMetadataError;
use betterlite_models::full_model_metadata::{
    HelperQuery, HelperSources, get_model_info_helper_cached,
};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::model_metadata_resolution::hf_config::HfConfigClient;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::resolution::ResolutionContext;

use super::cache::DiscoveryCache;
use super::discovery::CatalogSnapshot;

// B2225's model info lookup as the router reads it: an owned field map, None for an unmapped name and the source's own error.
pub trait ModelMetadataSource: Sync {
    type Error: Error + Send + Sync + 'static;

    fn get_model_info(
        &self,
        model: &str,
        keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Self::Error>> + Send;
}

// The shared resolver behind the trait: litellm.get_model_info with no provider hint and no api_base.
pub struct HelperModelMetadata<'call, 'source, 'environment, Source, Transport> {
    pub sources: &'call HelperSources<'source, 'environment, Source, Transport>,
    pub cache: &'call ModelInfoCache,
}

impl<Source, Transport> ModelMetadataSource for HelperModelMetadata<'_, '_, '_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    type Error = FullMetadataError<Infallible>;

    async fn get_model_info(
        &self,
        model: &str,
        keys: &mut ModelCostKeys,
    ) -> Result<Option<JsonObject>, FullMetadataError<Infallible>> {
        let query = HelperQuery {
            model,
            custom_llm_provider: None,
            api_base: None,
        };
        let handle = get_model_info_helper_cached(&query, self.sources, keys, self.cache).await?;
        let info = match handle.lock() {
            Ok(guard) => guard,
            Err(poisoned) => PoisonError::into_inner(poisoned),
        };
        Ok(Some(info.fields.clone()))
    }
}

// Everything a router metadata read needs beyond the deployment it was handed.
pub struct RouterMetadata<'call, 'catalog, 'resolution, Source> {
    pub snapshot: &'call CatalogSnapshot<'catalog>,
    pub discovery: &'call DiscoveryCache,
    pub registry: &'call ModelRegistry,
    pub resolution: &'call ResolutionContext<'resolution>,
    pub source: &'call Source,
}
