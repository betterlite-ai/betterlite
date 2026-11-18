use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::limits::IoLimits;

use super::error::ModelRateLimitError;

// ModelRateLimitingCheck._get_deployment_limits: the tpm and rpm a deployment sets, wherever the config put them
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModelLimits {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
}

pub fn model_limits(deployment: &DeploymentSpec) -> Result<ModelLimits, ModelRateLimitError> {
    Ok(ModelLimits {
        tpm: limit(deployment, "tpm")?,
        rpm: limit(deployment, "rpm")?,
    })
}

// get_deployment_io_token_limits reads the itpm/otpm pair from the same three places
pub fn io_limits(deployment: &DeploymentSpec) -> Result<IoLimits, ModelRateLimitError> {
    Ok(IoLimits {
        itpm: limit(deployment, "itpm")?,
        otpm: limit(deployment, "otpm")?,
    })
}

// the top-level deployment field wins over litellm_params, which wins over model_info
fn limit(
    deployment: &DeploymentSpec,
    field: &'static str,
) -> Result<Option<i64>, ModelRateLimitError> {
    for source in [
        &deployment.extra,
        &deployment.litellm_params,
        &deployment.model_info,
    ] {
        match source.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Number(number)) => {
                return number
                    .as_i64()
                    .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
                    .map(Some)
                    .ok_or(ModelRateLimitError::Limit { field });
            }
            Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                return Err(ModelRateLimitError::Limit { field });
            }
        }
    }
    Ok(None)
}

// _get_cache_keys reads model_info.id and litellm_params.model, and an absent one counts under the text None
#[must_use]
pub fn model_id(deployment: &DeploymentSpec) -> Option<&str> {
    match deployment.model_info.get("id") {
        Some(Json::Text(value)) => Some(value),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => None,
    }
}

#[must_use]
pub fn model(deployment: &DeploymentSpec) -> Option<&str> {
    match deployment.litellm_params.get("model") {
        Some(Json::Text(value)) => Some(value),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => None,
    }
}
