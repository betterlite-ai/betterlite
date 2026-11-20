use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_model_info::deployment_info::get_deployment_model_info;
use crate::deployment_model_info::metadata::{ModelMetadataSource, RouterMetadata};

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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

pub struct ProviderRequest<'model> {
    pub model: &'model str,
    pub custom_llm_provider: Option<&'model str>,
}

pub trait ProviderResolver {
    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure>;
}

// The collaborators one aggregation reads. The catalog stays out: its rows are read before the future is built.
pub struct GroupSources<'sources, D, R, P> {
    pub deployment_info: &'sources D,
    pub providers: &'sources R,
    pub parameters: &'sources P,
    pub cost_map: &'sources ModelCostMap,
    pub fast_mode_catalog: &'sources RawCatalog,
}

impl<Source> DeploymentInfo for RouterMetadata<'_, '_, '_, Source>
where
    Source: ModelMetadataSource,
{
    type Error = Infallible;

    async fn info(
        &self,
        model_id: &str,
        model_name: &str,
        keys: &mut ModelCostKeys,
    ) -> Result<Option<JsonObject>, Infallible> {
        Ok(get_deployment_model_info(self, model_id, model_name, keys).await)
    }
}
