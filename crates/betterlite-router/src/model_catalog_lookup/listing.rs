use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::scope::all_deployments;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::fields::text;

// Router.should_include_deployment: a team's public name, else the internal name when the caller has no team, the row is global, or the teams match.
#[must_use]
pub fn should_include_deployment(
    model_name: &str,
    deployment: &DeploymentSpec,
    team_id: Option<&str>,
) -> bool {
    let owner = text(&deployment.model_info, "team_id");
    let public_name = text(&deployment.model_info, "team_public_model_name");
    let named_by_team =
        team_id.is_some_and(|team| owner == Some(team)) && public_name == Some(model_name);
    if named_by_team {
        return true;
    }
    if deployment.model_name != model_name {
        return false;
    }
    team_id.is_none_or(|team| owner.is_none_or(|owner_id| owner_id == team))
}

// Router._get_all_deployments: the team rows first and only when some survive, then the name rows, then a scan when the name is unindexed; model_alias rewrites only the returned copies.
#[must_use]
pub fn get_all_deployments(
    catalog: &DeploymentCatalog,
    model_name: &str,
    model_alias: Option<&str>,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    let mut rows = all_deployments(catalog, model_name, team_id);
    if let Some(alias) = model_alias {
        for row in &mut rows {
            alias.clone_into(&mut row.model_name);
        }
    }
    rows
}

// Router._get_team_specific_model: the public name of a deployment the caller's team owns.
#[must_use]
pub fn team_specific_model<'info>(
    model_info: &'info JsonObject,
    team_id: Option<&str>,
) -> Option<&'info str> {
    if team_id != text(model_info, "team_id") {
        return None;
    }
    text(model_info, "team_public_model_name")
}
