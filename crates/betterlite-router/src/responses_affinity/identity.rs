use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;

// _find_deployment_by_model_id compares str(model_info["id"]), so an origin "7" matches a numeric id 7.
#[must_use]
pub fn deployment_id(deployment: &DeploymentSpec) -> Option<String> {
    match deployment.model_info.get("id") {
        Some(Json::Text(id)) => Some(id.clone()),
        Some(Json::Number(id)) => Some(id.to_string()),
        None | Some(Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

// _find_deployment_by_model_id: the first healthy deployment carrying the marker's id, malformed entries skipped.
#[must_use]
pub fn find_by_model_id<'deployments>(
    deployments: &'deployments [DeploymentSpec],
    model_id: &str,
) -> Option<&'deployments DeploymentSpec> {
    deployments
        .iter()
        .find(|deployment| deployment_id(deployment).as_deref() == Some(model_id))
}
