use betterlite_models::decisions::Json;

use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::types::Authority;
use crate::deployment_credentials::selection::deployment_usable_by_team;
use crate::deployment_identity::spec::DeploymentSpec;

// filter_team_based_models: a team-owned deployment serves only its own team, and an admin with no team is refused an ambiguous name.
pub fn filter_team_based_models(
    deployments: Vec<DeploymentSpec>,
    authority: Authority<'_>,
    requested_model: &str,
) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
    let team_id = authority.team_id.filter(|team| !team.is_empty());
    if team_id.is_none() && authority.proxy_admin && named_by_team(&deployments, requested_model) {
        if distinct_teams(&deployments) > 1 {
            return Err(CatalogLookupError::AmbiguousTeams {
                model: requested_model.to_owned(),
            });
        }
        return Ok(deployments);
    }
    // LiteLLM removes every deployment sharing the id of one that belongs to another team
    let removed: Vec<Option<String>> = deployments
        .iter()
        .filter(|deployment| !deployment_usable_by_team(deployment, team_id))
        .map(|deployment| deployment.id().map(str::to_owned))
        .collect();
    Ok(deployments
        .into_iter()
        .filter(|deployment| {
            !removed
                .iter()
                .any(|gone| gone.as_deref() == deployment.id())
        })
        .collect())
}

// The admin arm: every candidate belongs to a team and answers to the requested model under its name or public name.
fn named_by_team(deployments: &[DeploymentSpec], requested_model: &str) -> bool {
    !deployments.is_empty()
        && deployments.iter().all(|deployment| {
            deployment
                .model_info
                .get("team_id")
                .is_some_and(|team| !matches!(team, Json::Null))
                && (deployment.model_name == requested_model
                    || matches!(
                        deployment.model_info.get("team_public_model_name"),
                        Some(Json::Text(public)) if public == requested_model
                    ))
        })
}

fn distinct_teams(deployments: &[DeploymentSpec]) -> usize {
    let mut teams: Vec<&Json> = Vec::new();
    for deployment in deployments {
        if let Some(team) = deployment
            .model_info
            .get("team_id")
            .filter(|team| !matches!(team, Json::Null))
            && !teams.contains(&team)
        {
            teams.push(team);
        }
    }
    teams.len()
}
