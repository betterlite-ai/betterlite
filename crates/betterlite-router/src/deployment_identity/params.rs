use betterlite_models::call_helpers::generic_params::{
    GenericParamsError, normalize_generic_params,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::InvalidDeployment;

// LiteLLM_Params.model: a deployment and a stored DB row both need a model that is a string.
pub fn require_model(params: &JsonObject) -> Result<(), InvalidDeployment> {
    if !matches!(params.get("model"), Some(Json::Text(_))) {
        return Err(InvalidDeployment::MissingModel);
    }
    Ok(())
}

// the shared GenericLiteLLMParams dump's failures as the router's own, for LiteLLM_Params and the stored rows
#[must_use]
pub fn generic_params_failure(failure: GenericParamsError) -> InvalidDeployment {
    match failure {
        GenericParamsError::MaxRetries { value, source } => {
            InvalidDeployment::MaxRetries { value, source }
        }
        GenericParamsError::Affinity(cause) => InvalidDeployment::AffinityHeader(cause),
    }
}

// LiteLLM_Params(**raw): the required model, then GenericLiteLLMParams.model_dump(exclude_none=True)
pub fn litellm_params(raw: &JsonObject) -> Result<JsonObject, InvalidDeployment> {
    require_model(raw)?;
    normalize_generic_params(raw).map_err(generic_params_failure)
}
