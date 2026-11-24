use betterlite_auth::auth_model_lists::team_models::get_team_models;
use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use indexmap::{IndexMap, IndexSet};

use super::error::ScopeError;
use super::text_field;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamScope {
    pub team_id: String,
    pub models: Vec<String>,
    pub access_group_ids: Vec<String>,
}

pub type TeamModelMap = IndexMap<String, IndexSet<String>>;
pub type TeamModelLists = IndexMap<String, Vec<String>>;

#[must_use]
pub fn check_if_model_is_team_model(
    models: &[DeploymentSpec],
    teams: &[String],
) -> Vec<DeploymentSpec> {
    models
        .iter()
        .filter(|model| {
            text_field(&model.model_info, "team_id")
                .is_some_and(|team| teams.iter().any(|member| member == team))
        })
        .cloned()
        .collect()
}

fn open(team: &TeamScope) -> bool {
    if team.models.is_empty() {
        return true;
    }
    team.models.iter().any(|model| model == ALL_PROXY_MODELS)
}

fn mark(team_models: &mut TeamModelMap, team_id: &str, model_id: &str) {
    team_models
        .entry(model_id.to_owned())
        .or_default()
        .insert(team_id.to_owned());
}

fn add_unrestricted_team(
    team: &TeamScope,
    lookup: &CatalogLookup<'_>,
    team_models: &mut TeamModelMap,
) -> Result<(), ScopeError> {
    for deployment in lookup.get_model_list(None, None)? {
        let Some(model_id) = deployment.id() else {
            continue;
        };
        let reachable = text_field(&deployment.model_info, "team_id")
            .is_none_or(|model_team| team.team_id.contains(model_team));
        if reachable {
            mark(team_models, &team.team_id, model_id);
        }
    }
    Ok(())
}

fn record_rows<I>(
    team: &TeamScope,
    model_names: I,
    lookup: &CatalogLookup<'_>,
    team_models: &mut TeamModelMap,
) -> Result<(), ScopeError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    for model_name in model_names {
        for deployment in lookup.get_model_list(Some(model_name.as_ref()), Some(&team.team_id))? {
            let Some(model_id) = deployment.id() else {
                continue;
            };
            mark(team_models, &team.team_id, model_id);
        }
    }
    Ok(())
}

pub fn add_team_models_to_all_models(
    teams: &[TeamScope],
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<TeamModelMap, ScopeError> {
    let proxy_model_list = lookup.get_model_names(None)?;
    let groups = access_groups.get_model_access_groups(lookup, None, None, None)?;
    let mut team_models = TeamModelMap::new();
    for team in teams {
        if open(team) {
            add_unrestricted_team(team, lookup, &mut team_models)?;
        } else {
            let names = get_team_models(&team.models, &proxy_model_list, &groups, false);
            record_rows(team, names, lookup, &mut team_models)?;
        }
    }
    Ok(team_models)
}
#[must_use]
pub fn access_group_ids_for_teams(teams: &[TeamScope]) -> Vec<String> {
    let mut ids = IndexSet::new();
    for team in teams {
        if team.access_group_ids.is_empty() || open(team) {
            continue;
        }
        ids.extend(team.access_group_ids.iter().cloned());
    }
    ids.into_iter().collect()
}

pub fn add_access_group_models(
    lookup: &CatalogLookup<'_>,
    teams: &[TeamScope],
    group_models: &AccessGroupMap,
    team_models: &mut TeamModelMap,
) -> Result<(), ScopeError> {
    for team in teams {
        if team.access_group_ids.is_empty() || open(team) {
            continue;
        }
        let mut model_names = IndexSet::new();
        for group_id in &team.access_group_ids {
            if let Some(models) = group_models.get(group_id) {
                model_names.extend(models.iter().map(String::as_str));
            }
        }
        record_rows(team, model_names, lookup, team_models)?;
    }
    Ok(())
}
