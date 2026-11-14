use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_identity::spec::DeploymentSpec;

fn info_text<'spec>(spec: &'spec DeploymentSpec, key: &str) -> Option<&'spec str> {
    match spec.model_info.get(key) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}

// The litellm_params.model of a spec; an entry without a text model reads as empty, as the catalog indexes it.
#[must_use]
pub fn litellm_model(spec: &DeploymentSpec) -> &str {
    match spec.litellm_params.get("model") {
        Some(Json::Text(model)) => model,
        _ => "",
    }
}

// Router.should_include_deployment
fn visible(name: &str, spec: &DeploymentSpec, team_id: Option<&str>) -> bool {
    let owner = info_text(spec, "team_id");
    if let Some(team) = team_id
        && owner == Some(team)
        && info_text(spec, "team_public_model_name") == Some(name)
    {
        return true;
    }
    spec.model_name == name && (team_id.is_none() || owner.is_none() || owner == team_id)
}

// Router._get_all_deployments: the team's own public name first, then the model name, then a scan for a team caller.
#[must_use]
pub fn all_deployments(
    catalog: &DeploymentCatalog,
    name: &str,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    let pick = |indices: &[usize]| -> Vec<DeploymentSpec> {
        indices
            .iter()
            .filter_map(|index| catalog.models.get(*index))
            .filter(|spec| visible(name, spec, team_id))
            .cloned()
            .collect()
    };
    if let Some(indices) =
        team_id.and_then(|team| catalog.team_index.get(&(team.to_owned(), name.to_owned())))
    {
        let owned = pick(indices);
        if !owned.is_empty() {
            return owned;
        }
    }
    match catalog.name_index.get(name) {
        Some(indices) => pick(indices),
        None if team_id.is_some() => catalog
            .models
            .iter()
            .filter(|spec| visible(name, spec, team_id))
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}

// Router._team_deployments_across_teams
pub fn across_teams(
    catalog: &DeploymentCatalog,
    name: &str,
) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
    let owned: Vec<&Vec<usize>> = catalog
        .team_index
        .iter()
        .filter(|((_, public_name), _)| public_name == name)
        .map(|(_, indices)| indices)
        .collect();
    if owned.len() > 1 {
        return Err(CatalogLookupError::AmbiguousTeams {
            model: name.to_owned(),
        });
    }
    Ok(owned
        .into_iter()
        .flatten()
        .filter_map(|index| catalog.models.get(*index))
        .cloned()
        .collect())
}
