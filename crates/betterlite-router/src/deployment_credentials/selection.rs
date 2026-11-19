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

// A lookup's answer: the deployment it selected, or the typed error a malformed candidate earns.
type Lookup = Result<Option<SelectedDeployment>, CredentialError>;

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
    substitute: Option<&str>,
) -> Result<SelectedDeployment, CredentialError> {
    let mut deployment = row.clone();
    if let Some(model) = substitute {
        deployment
            .litellm_params
            .insert("model".to_owned(), Json::Text(model.to_owned()));
    }
    let read = Deployment::read(&deployment.litellm_params, &deployment.model_info)?;
    let model = read.target.model.to_owned();
    Ok(SelectedDeployment { deployment, model })
}

// Deployment(**candidate): a wildcard candidate keeps the recorded row's credentials under the substituted model.
fn candidate_row(candidate: &Candidate, row: Option<DeploymentSpec>) -> Lookup {
    row.map_or_else(
        || Ok(None),
        |served| materialize(&served, Some(&candidate.model)).map(Some),
    )
}

// The row an index names, or the typed error a stale index earns.
fn row_at(catalog: &DeploymentCatalog, index: usize) -> Result<&DeploymentSpec, CredentialError> {
    catalog
        .models
        .get(index)
        .ok_or(CredentialError::InvalidRow { index })
}

// Router.get_deployment: the id map only, so an exact id never runs the team predicate and authorization stays with the caller.
pub fn deployment_by_id(catalog: &DeploymentCatalog, model_id: &str) -> Lookup {
    let Some(index) = catalog.id_index.get(model_id) else {
        return Ok(None);
    };
    materialize(row_at(catalog, *index)?, None).map(Some)
}

// _get_model_group_deployment_usable_by_team, then _get_team_public_name_deployment, then the wildcard lists.
fn by_name_or_pattern(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Lookup {
    for index in catalog.name_index.get(model_id).into_iter().flatten() {
        let row = row_at(catalog, *index)?;
        if deployment_usable_by_team(row, team_id) {
            return materialize(row, None).map(Some);
        }
    }
    let key = team_id.map(|team| (team.to_owned(), model_id.to_owned()));
    let index = key
        .as_ref()
        .and_then(|pair| catalog.team_index.get(pair))
        .and_then(|indices| indices.first());
    if let Some(&first) = index {
        return materialize(row_at(catalog, first)?, None).map(Some);
    }
    wildcard_deployment(catalog, model_id, team_id)
}

// _get_wildcard_deployment_usable_by_team: the team's own wildcard list first, then the global one filtered by owner.
fn wildcard_deployment(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Lookup {
    let team_rows = team_id
        .and_then(|team| catalog.team_pattern_routers.get(team))
        .and_then(|patterns| patterns.router.get_pattern_rows(model_id, None));
    if let Some(rows) = team_rows {
        let Some((candidate, row)) = rows.into_iter().next() else {
            return Ok(None);
        };
        return candidate_row(&candidate, row);
    }
    let matched = catalog
        .pattern_router
        .get_pattern_rows(model_id, None)
        .unwrap_or_default()
        .into_iter()
        .find(|(_, deployment)| {
            deployment
                .as_ref()
                .is_none_or(|served| deployment_usable_by_team(served, team_id))
        });
    let Some((candidate, row)) = matched else {
        return Ok(None);
    };
    candidate_row(&candidate, row)
}

// Router.get_credential_deployment: the exact id, then the model name, then the team's public name, then a wildcard.
pub fn credential_deployment(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Lookup {
    let selected = match deployment_by_id(catalog, model_id)? {
        Some(exact) => Some(exact),
        None => by_name_or_pattern(catalog, model_id, team_id)?,
    };
    // the blocked test runs once the winner is chosen: a paused first match never falls through
    Ok(selected.filter(|chosen| !is_deployment_blocked(&chosen.deployment)))
}
