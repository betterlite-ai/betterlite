use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::lookup::early_deployments;
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::index::deployment_ids;
use crate::model_catalog_lookup::listing::{
    get_all_deployments, is_team_specific_model, team_specific_model,
};
use crate::routing_groups::index::RoutingGroupIndex;
use crate::routing_groups::priority::prioritized;

// The snapshot and the collaborator maps every catalog lookup reads.
pub struct CatalogLookup<'catalog> {
    pub catalog: &'catalog DeploymentCatalog,
    pub model_group_alias: &'catalog AliasMap,
    pub routing_groups: &'catalog RoutingGroupIndex,
    // inferred_provider, the provider prefix a wildcard lookup tries after the bare name
    pub inferred_provider: &'catalog dyn Fn(&str) -> Option<String>,
}

// The row an index names, or the typed failure when that index points past the model list.
fn row_at(catalog: &DeploymentCatalog, index: usize) -> Result<&DeploymentSpec, LookupError> {
    catalog
        .models
        .get(index)
        .ok_or(LookupError::InvalidRow { index })
}

// The id filter both id listings share: a row that lost its id field and, when asked, a team row are left out.
fn listed_id(row: &DeploymentSpec, exclude_team_models: bool) -> bool {
    row.model_info.contains_key("id")
        && !(exclude_team_models && is_team_specific_model(&row.model_info))
}

impl CatalogLookup<'_> {
    // Router.get_deployment: the id map only, which answers none for a miss.
    pub fn get_deployment(&self, model_id: &str) -> Result<Option<&DeploymentSpec>, LookupError> {
        match self.catalog.id_index.get(model_id) {
            Some(index) => row_at(self.catalog, *index).map(Some),
            None => Ok(None),
        }
    }

    // Router.get_deployment_by_model_group_name: the first sibling the name index lists, or none for a missing or empty index.
    pub fn deployment_by_group_name(
        &self,
        model_group_name: &str,
    ) -> Result<Option<&DeploymentSpec>, LookupError> {
        self.catalog
            .name_index
            .get(model_group_name)
            .and_then(|indices| indices.first())
            .map(|index| row_at(self.catalog, *index))
            .transpose()
    }

    // Router.get_model_ids: the named index's rows with their own ids, or the id map's keys in its own order, where an entry that lost its row drops out like a stale one does.
    #[must_use]
    pub fn get_model_ids(&self, model_name: Option<&str>, exclude_team_models: bool) -> Vec<Json> {
        let Some(name) = model_name else {
            return self
                .catalog
                .id_index
                .iter()
                .filter_map(|(id, index)| Some((id, self.catalog.models.get(*index)?)))
                .filter(|(_, row)| listed_id(row, exclude_team_models))
                .map(|(id, _)| Json::Text(id.clone()))
                .collect();
        };
        let Some(indices) = self.catalog.name_index.get(name) else {
            return Vec::new();
        };
        indices
            .iter()
            .filter_map(|index| self.catalog.models.get(*index))
            .filter(|row| listed_id(row, exclude_team_models))
            .filter_map(|row| row.model_info.get("id").cloned())
            .collect()
    }

    // Router.get_candidate_model_ids_for_route: the alias, then a routing group even when it is empty, then the first early route, then the name and team indices.
    pub fn candidate_model_ids(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<BTreeSet<String>, LookupError> {
        let resolved = self
            .model_group_alias
            .get(model)
            .and_then(AliasTarget::resolved)
            .unwrap_or(model);
        if let Some(members) = self.routing_group_deployments(resolved, team_id) {
            return Ok(deployment_ids(&members));
        }
        if let Some(early) = early_deployments(
            self.catalog,
            self.inferred_provider,
            resolved,
            team_id,
            false,
        )? {
            return Ok(deployment_ids(&early));
        }
        Ok(deployment_ids(&get_all_deployments(
            self.catalog,
            resolved,
            None,
            team_id,
        )))
    }

    // Router._get_routing_group_deployments: the union over the group's members, or none when the name is not a callable group.
    fn routing_group_deployments(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Option<Vec<DeploymentSpec>> {
        let group = self.routing_groups.callable_group(model)?;
        let mut members = Vec::new();
        for member in &group.models {
            for deployment in get_all_deployments(self.catalog, member, None, team_id) {
                members.push(prioritized(group, &deployment));
            }
        }
        Some(members)
    }

    // Router.resolve_model_name_from_model_id: the public name, then the id, then a provider model that ends in the id.
    pub fn resolve_model_name(
        &self,
        model_id: Option<&str>,
    ) -> Result<Option<String>, LookupError> {
        let Some(model_id) = model_id.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        if self.catalog.model_names.contains(model_id) {
            return Ok(Some(model_id.to_owned()));
        }
        if self.catalog.id_index.contains_key(model_id) {
            let named = self
                .get_deployment(model_id)?
                .map(|deployment| deployment.model_name.as_str())
                .filter(|name| !name.is_empty());
            return Ok(Some(named.unwrap_or(model_id).to_owned()));
        }
        for deployment in self.get_model_list(None, None)? {
            if provider_model_matches(litellm_model(&deployment), model_id)
                && !deployment.model_name.is_empty()
            {
                return Ok(Some(deployment.model_name));
            }
        }
        Ok(None)
    }

    // Router.get_model_names: team public names for a matching caller, and the internal name of every global row.
    pub fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupError> {
        let mut names = Vec::new();
        for deployment in self.get_model_list(None, None)? {
            if is_team_specific_model(&deployment.model_info) {
                if let Some(name) = team_specific_model(&deployment.model_info, team_id)
                    .filter(|name| !name.is_empty())
                {
                    names.push(name.to_owned());
                }
            } else {
                names.push(deployment.model_name.clone());
            }
        }
        Ok(names)
    }
}

// The provider-model arm of resolve_model_name: the exact value, a '/id' or a ':id' suffix, case kept.
fn provider_model_matches(actual: &str, model_id: &str) -> bool {
    actual == model_id
        || actual.ends_with(&format!("/{model_id}"))
        || actual.ends_with(&format!(":{model_id}"))
}
