use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_credentials::error::CredentialError;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_pricing::deployment::Deployment;
use crate::wildcard_catalog::Candidate;

// The deployment a lookup selected, beside the model its litellm_params names after a wildcard substitution.
#[derive(Clone, Debug)]
pub struct SelectedDeployment {
    pub deployment: DeploymentSpec,
    pub model: String,
}

// _deployment_usable_by_team: an unowned deployment is shared, and a team-owned one serves only that same team.
#[must_use]
pub fn deployment_usable_by_team(deployment: &DeploymentSpec, team_id: Option<&str>) -> bool {
    match deployment.model_info.get("team_id") {
        None | Some(Json::Null) => true,
        Some(Json::Text(owner)) => team_id == Some(owner.as_str()),
        Some(_) => false,
    }
}

// _is_deployment_blocked: only the boolean true pauses a deployment; a missing, null or false flag does not.
#[must_use]
pub fn is_deployment_blocked(deployment: &DeploymentSpec) -> bool {
    match deployment.model_info.get("blocked") {
        Some(Json::Bool(blocked)) => *blocked,
        _ => false,
    }
}

// Deployment(**row): the existing validator accepts the row, with the wildcard's model substituted when there is one.
fn materialize(
    row: &DeploymentSpec,
    model: Option<&str>,
) -> Result<SelectedDeployment, CredentialError> {
    let mut deployment = row.clone();
    if let Some(model) = model {
        deployment
            .litellm_params
            .insert("model".to_owned(), Json::Text(model.to_owned()));
    }
    let read = Deployment::read(&deployment.litellm_params, &deployment.model_info)?;
    let model = read.target.model.to_owned();
    Ok(SelectedDeployment { deployment, model })
}

// Deployment(**candidate): a wildcard candidate keeps the recorded row's credentials under the substituted model.
fn candidate_row(
    candidate: &Candidate,
    row: Option<DeploymentSpec>,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let Some(row) = row else {
        return Ok(None);
    };
    materialize(&row, Some(&candidate.model)).map(Some)
}

// Router.get_deployment: the id map only, so an exact id never runs the team predicate and authorization stays with the caller.
pub fn deployment_by_id(
    catalog: &DeploymentCatalog,
    model_id: &str,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let Some(index) = catalog.id_index.get(model_id) else {
        return Ok(None);
    };
    let row = catalog
        .models
        .get(*index)
        .ok_or(CredentialError::InvalidRow { index: *index })?;
    materialize(row, None).map(Some)
}

// _get_model_group_deployment_usable_by_team: the first row under the name owned by the caller or by nobody.
pub fn model_group_deployment(
    catalog: &DeploymentCatalog,
    model_name: &str,
    team_id: Option<&str>,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let Some(indices) = catalog.name_index.get(model_name) else {
        return Ok(None);
    };
    for index in indices {
        let row = catalog
            .models
            .get(*index)
            .ok_or(CredentialError::InvalidRow { index: *index })?;
        if deployment_usable_by_team(row, team_id) {
            return materialize(row, None).map(Some);
        }
    }
    Ok(None)
}

// _get_team_public_name_deployment: the first row the team index holds, and nothing at all for a caller without a team.
pub fn team_public_name_deployment(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let Some(team) = team_id else {
        return Ok(None);
    };
    let Some(indices) = catalog
        .team_index
        .get(&(team.to_owned(), model_id.to_owned()))
    else {
        return Ok(None);
    };
    let Some(index) = indices.first() else {
        return Ok(None);
    };
    let row = catalog
        .models
        .get(*index)
        .ok_or(CredentialError::InvalidRow { index: *index })?;
    materialize(row, None).map(Some)
}

// _get_wildcard_deployment_usable_by_team: the team's own wildcard list first, then the global one filtered by owner.
pub fn wildcard_deployment(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let team_rows = team_id
        .and_then(|team| catalog.team_pattern_routers.get(team))
        .and_then(|patterns| patterns.router.get_pattern_rows(model_id, None));
    if let Some(rows) = team_rows {
        let Some((candidate, row)) = rows.into_iter().next() else {
            return Ok(None);
        };
        return candidate_row(&candidate, row);
    }
    let global = catalog
        .pattern_router
        .get_pattern_rows(model_id, None)
        .unwrap_or_default();
    let chosen = global.into_iter().find(|(_, row)| {
        row.as_ref()
            .is_none_or(|row| deployment_usable_by_team(row, team_id))
    });
    let Some((candidate, row)) = chosen else {
        return Ok(None);
    };
    candidate_row(&candidate, row)
}

// Router.get_credential_deployment: the exact id, then the model name, then the team's public name, then a wildcard.
pub fn credential_deployment(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Result<Option<SelectedDeployment>, CredentialError> {
    let mut selected = deployment_by_id(catalog, model_id)?;
    if selected.is_none() {
        selected = model_group_deployment(catalog, model_id, team_id)?;
    }
    if selected.is_none() {
        selected = team_public_name_deployment(catalog, model_id, team_id)?;
    }
    if selected.is_none() {
        selected = wildcard_deployment(catalog, model_id, team_id)?;
    }
    match selected {
        // the blocked test runs once the candidate is chosen: a paused first match never falls through
        Some(selected) if is_deployment_blocked(&selected.deployment) => Ok(None),
        other => Ok(other),
    }
}
