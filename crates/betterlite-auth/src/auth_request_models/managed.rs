use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::catalog::{
    CatalogError, DeploymentRecord, ManagedIdDecoders, ManagedIdFamily, ManagedIdField,
    ModelCatalog,
};
use super::dedupe_model_candidates;

#[must_use]
pub fn resolve_model_id_with_router(
    model_id: Option<String>,
    catalog: Option<&dyn ModelCatalog>,
) -> Option<String> {
    let requested = model_id?;
    let Some(known_catalog) = catalog else {
        return Some(requested);
    };
    match known_catalog.resolve_model_name(&requested) {
        Ok(Some(name)) if !name.is_empty() => Some(name),
        Ok(_) => Some(requested),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "unable to resolve model_id from managed resource ID"
            );
            Some(requested)
        }
    }
}

#[must_use]
pub fn managed_resource_models(
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
    let families: &[ManagedIdFamily] = match ManagedIdField::from_name(field) {
        ManagedIdField::Video => &[
            ManagedIdFamily::LegacyFileBatch,
            ManagedIdFamily::Unified,
            ManagedIdFamily::Video,
        ],
        ManagedIdField::Character => &[
            ManagedIdFamily::LegacyFileBatch,
            ManagedIdFamily::Unified,
            ManagedIdFamily::Character,
        ],
        ManagedIdField::Other => &[ManagedIdFamily::LegacyFileBatch, ManagedIdFamily::Unified],
    };
    let mut candidates = Vec::new();
    for family in families {
        if let Err(error) = decoders.decode_family(*family, resource_id, catalog, &mut candidates) {
            tracing::debug!(
                error = %error,
                family = ?family,
                "unable to extract a model from a managed resource ID"
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

pub fn prediction_deployments(
    current_deployment_id: &str,
    candidate_deployment_id: &str,
    catalog: Option<&dyn ModelCatalog>,
    team_id: Option<&str>,
) -> Result<Option<(DeploymentRecord, DeploymentRecord)>, CatalogError> {
    let Some(known_catalog) = catalog else {
        return Ok(None);
    };
    let Some(current) = known_catalog.deployment(current_deployment_id)? else {
        return Ok(None);
    };
    let Some(candidate) = known_catalog.deployment(candidate_deployment_id)? else {
        return Ok(None);
    };
    if !team_can_use(&current, team_id) || !team_can_use(&candidate, team_id) {
        return Ok(None);
    }
    Ok(Some((current, candidate)))
}

pub fn prediction_candidates(
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
        prediction_deployments(current_id, candidate_id, catalog, team_id)?
    else {
        return Ok(Vec::new());
    };
    Ok(vec![current.model_name, candidate.model_name])
}
