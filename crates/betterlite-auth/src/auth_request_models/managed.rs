use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::catalog::{
    CatalogError, DeploymentRecord, ManagedIdDecoders, ManagedIdField, ModelCatalog,
};
use super::dedupe_model_candidates;

#[must_use]
pub fn resolve_model_id_with_router(
    model_id: Option<String>,
    catalog: Option<&dyn ModelCatalog>,
) -> Option<String> {
    let model_id = model_id?;
    let Some(catalog) = catalog else {
        return Some(model_id);
    };
    match catalog.resolve_model_name_from_model_id(&model_id) {
        Ok(Some(name)) if !name.is_empty() => Some(name),
        Ok(_resolved) => Some(model_id),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "unable to resolve model_id from managed resource ID"
            );
            Some(model_id)
        }
    }
}

pub fn extract_models_from_managed_resource_id(
    value: &Json,
    field: &str,
    catalog: Option<&dyn ModelCatalog>,
    decoders: &dyn ManagedIdDecoders,
) -> Vec<String> {
    let Json::Text(resource_id) = value else {
        return Vec::new();
    };
    if resource_id.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Err(error) = decoders.legacy_file_batch(resource_id, catalog, &mut candidates) {
        tracing::debug!(
            error = %error,
            "unable to extract model from managed file/batch ID"
        );
    }
    if let Err(error) = decoders.unified(resource_id, catalog, &mut candidates) {
        tracing::debug!(
            error = %error,
            "unable to extract model from unified managed resource ID"
        );
    }
    let field_kind = ManagedIdField::from_name(field);
    if field_kind != ManagedIdField::Other {
        if let Err(error) =
            decoders.video_character(resource_id, field_kind, catalog, &mut candidates)
        {
            tracing::debug!(
                error = %error,
                "unable to extract model from managed video/character ID"
            );
        }
    }
    dedupe_model_candidates(&candidates)
}

fn team_can_use(deployment: &DeploymentRecord, team_id: Option<&str>) -> bool {
    deployment
        .team_id
        .as_deref()
        .is_none_or(|team| Some(team) == team_id)
}

pub fn get_cache_prediction_deployments(
    current_deployment_id: &str,
    candidate_deployment_id: &str,
    catalog: Option<&dyn ModelCatalog>,
    team_id: Option<&str>,
) -> Result<Option<(DeploymentRecord, DeploymentRecord)>, CatalogError> {
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let Some(current) = catalog.deployment(current_deployment_id)? else {
        return Ok(None);
    };
    let Some(candidate) = catalog.deployment(candidate_deployment_id)? else {
        return Ok(None);
    };
    if !team_can_use(&current, team_id) || !team_can_use(&candidate, team_id) {
        return Ok(None);
    }
    Ok(Some((current, candidate)))
}

pub fn cache_prediction_model_candidates(
    request_data: &JsonObject,
    catalog: Option<&dyn ModelCatalog>,
    team_id: Option<&str>,
) -> Result<Vec<String>, CatalogError> {
    let Some(Json::Text(current_id)) = request_data.get("current_deployment_id") else {
        return Ok(Vec::new());
    };
    let Some(Json::Text(candidate_id)) = request_data.get("candidate_deployment_id") else {
        return Ok(Vec::new());
    };
    let Some((current, candidate)) =
        get_cache_prediction_deployments(current_id, candidate_id, catalog, team_id)?
    else {
        return Ok(Vec::new());
    };
    Ok(vec![current.model_name, candidate.model_name])
}
