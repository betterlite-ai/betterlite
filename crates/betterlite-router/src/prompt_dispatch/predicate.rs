use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;

// Router._is_prompt_management_model: exactly one deployment, a model with a slash, and a known prefix.
#[must_use]
pub fn is_prompt_model(deployments: &[DeploymentSpec], known: &[&str]) -> bool {
    let [deployment] = deployments else {
        return false;
    };
    let Some(Json::Text(model)) = deployment.litellm_params.get("model") else {
        return false;
    };
    let Some((prefix, _)) = model.split_once('/') else {
        return false;
    };
    known.contains(&prefix)
}
