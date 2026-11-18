use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::lookup::pattern_deployments;
use crate::deployment_catalog::scope::all_deployments;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasTarget;
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::fields::text;
use crate::model_catalog_lookup::lookup::CatalogLookup;

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
    match team_id {
        None => true,
        Some(team) => owner.is_none_or(|owner| owner == team),
    }
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

// Router._is_team_specific_model: a deployment names a team when its team_id is non-empty text.
#[must_use]
pub fn is_team_specific_model(model_info: &JsonObject) -> bool {
    match text(model_info, "team_id") {
        Some(team) => !team.is_empty(),
        None => false,
    }
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

impl CatalogLookup<'_> {
    // Router.get_model_list_from_model_alias: one alias for a named lookup, every alias in insertion order otherwise.
    pub fn list_from_alias(
        &self,
        model_name: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupError> {
        let targets: Vec<(&String, &AliasTarget)> = match model_name {
            Some(name) => self
                .model_group_alias
                .get_key_value(name)
                .into_iter()
                .collect(),
            None => self.model_group_alias.iter().collect(),
        };
        let mut returned = Vec::new();
        for (alias, target) in targets {
            let Some(model) = target.listed_model(alias, model_name.is_none())? else {
                continue;
            };
            if self.routing_groups.callable_group(model).is_some() {
                for mut row in self.routing_groups.rows(Some(model), &|member| {
                    get_all_deployments(self.catalog, member, None, None)
                }) {
                    alias.clone_into(&mut row.model_name);
                    returned.push(row);
                }
            } else {
                returned.extend(get_all_deployments(
                    self.catalog,
                    model,
                    Some(alias.as_str()),
                    None,
                ));
            }
        }
        Ok(returned)
    }

    // Router.get_model_list: the exact and team rows, the alias rows, the routing-group rows, then the wildcard rows.
    pub fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupError> {
        let mut returned = Vec::new();
        if let Some(name) = model_name {
            returned.extend(get_all_deployments(self.catalog, name, None, team_id));
        }
        returned.extend(self.list_from_alias(model_name)?);
        returned.extend(self.routing_groups.rows(model_name, &|member| {
            get_all_deployments(self.catalog, member, None, None)
        }));
        // the wildcard channel runs only when no other channel found the requested name, and the rows it finds are named after the request
        if returned.is_empty()
            && let Some(name) = model_name
        {
            let mut found =
                pattern_deployments(self.catalog, self.inferred_provider, name, team_id)?;
            for row in &mut found {
                name.clone_into(&mut row.model_name);
            }
            returned.extend(found);
        }
        if model_name.is_none() {
            returned.extend(self.catalog.models.iter().cloned());
        }
        Ok(returned)
    }
}
