use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::logging_metadata::params::LitellmParams;

pub type ModelInfo = IndexMap<String, Json>;

#[derive(Debug, Error)]
pub enum RouterModelError {
    #[error("litellm_params.model_info must be an object")]
    ModelInfoType,
    #[error("litellm_params.model_info.id must be a string")]
    ModelIdType,
}

// The router prices a deployment under the id it registered, and it keeps that under litellm_metadata
pub fn get_router_model_id(params: &LitellmParams) -> Result<Option<String>, RouterModelError> {
    let sources = [params.litellm_metadata.as_ref(), params.metadata.as_ref()];
    for source in sources.into_iter().flatten() {
        let model_info = match source.get("model_info") {
            None | Some(Json::Null) => continue,
            Some(Json::Object(model_info)) => model_info,
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(RouterModelError::ModelInfoType);
            }
        };
        match model_info.get("id") {
            None | Some(Json::Null) => {}
            Some(Json::Text(id)) => return Ok(Some(id.clone())),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(RouterModelError::ModelIdType);
            }
        }
    }
    Ok(None)
}

// pricing runs in betterlite-core, which depends on this crate, so it arrives as a dependency
pub trait DeploymentPricing {
    fn deployment_pricing_model_info(
        &self,
        model_id: Option<&str>,
        deployment_model: Option<&str>,
    ) -> Option<ModelInfo>;
}

pub fn router_deployment_model_info(
    pricing: &dyn DeploymentPricing,
    params: &LitellmParams,
    deployment_model: Option<&str>,
) -> Result<Option<ModelInfo>, RouterModelError> {
    Ok(pricing
        .deployment_pricing_model_info(get_router_model_id(params)?.as_deref(), deployment_model))
}

mod tests;
