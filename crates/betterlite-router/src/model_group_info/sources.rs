use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_model_info::deployment_info::get_deployment_model_info;
use crate::deployment_model_info::metadata::{ModelMetadataSource, RouterMetadata};
use crate::model_catalog_lookup::lookup::CatalogLookup;

// get_deployment_model_info behind a trait, so the aggregation can be tested and read by other crates without a full router.
pub trait DeploymentInfo {
    type Error: Error + 'static;

    fn info(
        &self,
        model_id: &str,
        model_name: &str,
        keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Self::Error>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderAnswer {
    pub litellm_model: String,
    pub provider: String,
}

// A provider lookup failure, split the way _set_model_group_info treats it: BadRequest is logged, the rest escape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderFailure {
    BadRequest(String),
    Other(String),
}

pub struct ProviderRequest<'a> {
    pub model: &'a str,
    pub custom_llm_provider: Option<&'a str>,
}

pub trait ProviderResolver {
    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure>;
}

// Everything one get_model_group_info call reads, so the router owns the inputs and the module owns the aggregation.
pub struct GroupSources<'a, D, R, P> {
    pub lookup: &'a CatalogLookup<'a>,
    pub deployment_info: &'a D,
    pub providers: &'a R,
    pub parameters: &'a P,
    pub cost_map: &'a ModelCostMap,
    pub fast_mode_catalog: &'a RawCatalog,
}

impl<Source> DeploymentInfo for RouterMetadata<'_, '_, '_, Source>
where
    Source: ModelMetadataSource,
{
    type Error = Infallible;

    fn info(
        &self,
        model_id: &str,
        model_name: &str,
        keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        async move { Ok(get_deployment_model_info(self, model_id, model_name, keys).await) }
    }
}
