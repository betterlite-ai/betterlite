use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::index::deployment_ids;
use crate::model_catalog_lookup::listing::{
    get_all_deployments, is_team_specific_model, pattern_pool,
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

impl CatalogLookup<'_> {
    // Router.get_deployment: the id map only, which answers none for a miss.
    pub fn get_deployment(&self, model_id: &str) -> Result<Option<&DeploymentSpec>, LookupError> {
        let Some(index) = self.catalog.id_index.get(model_id) else {
            return Ok(None);
        };
        let deployment = self
            .catalog
            .models
            .get(*index)
            .ok_or(LookupError::InvalidRow { index: *index })?;
        Ok(Some(deployment))
    }

    // Router.get_deployment_by_model_group_name: the first sibling the name index lists, or none for a missing or empty index.
    pub fn get_deployment_by_model_group_name(
        &self,
        model_group_name: &str,
    ) -> Result<Option<&DeploymentSpec>, LookupError> {
        let Some(first) = self
            .catalog
            .name_index
            .get(model_group_name)
            .and_then(|indices| indices.first())
        else {
            return Ok(None);
        };
        let deployment = self
            .catalog
            .models
            .get(*first)
            .ok_or(LookupError::InvalidRow { index: *first })?;
        Ok(Some(deployment))
    }

    // Router.has_model_id: literal membership in the id map.
    #[must_use]
    pub fn has_model_id(&self, candidate_id: &str) -> bool {
        self.catalog.id_index.contains_key(candidate_id)
    }

    // Router.get_model_ids: the named index when a name is given, else the id map's insertion order.
    #[must_use]
    pub fn get_model_ids(&self, model_name: Option<&str>, exclude_team_models: bool) -> Vec<Json> {
        match model_name {
            Some(name) => self.named_model_ids(name, exclude_team_models),
            None => self.all_model_ids(exclude_team_models),
        }
    }

    fn named_model_ids(&self, name: &str, exclude_team_models: bool) -> Vec<Json> {
        let Some(indices) = self.catalog.name_index.get(name) else {
            return Vec::new();
        };
        indices
            .iter()
            .filter_map(|index| self.catalog.models.get(*index))
            .filter(|row| !(exclude_team_models && is_team_specific_model(&row.model_info)))
            .filter_map(|row| row.model_info.get("id").cloned())
            .collect()
    }

    // The id map's insertion order: a duplicate id keeps its first position and points at its last row.
    fn all_model_ids(&self, exclude_team_models: bool) -> Vec<Json> {
        let mut order: Vec<(String, bool)> = Vec::new();
        for row in &self.catalog.models {
            let Some(id) = row.id() else {
                continue;
            };
            let team = is_team_specific_model(&row.model_info);
            match order.iter_mut().find(|(key, _)| key == id) {
                Some(entry) => entry.1 = team,
                None => order.push((id.to_owned(), team)),
            }
        }
        order
            .into_iter()
            .filter(|(_, team)| !(exclude_team_models && *team))
            .map(|(id, _)| Json::Text(id))
            .collect()
    }

    // Router.get_candidate_model_ids_for_route: the alias, then a routing group even when it is empty, then the first early route, then the name and team indices.
    pub fn get_candidate_model_ids_for_route(
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
        if let Some(early) = self.early_deployments(resolved, team_id)? {
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

    // Router._try_early_resolve_deployments_for_model_not_in_names for a caller with one team: team deployments, then patterns, then the team's patterns.
    fn early_deployments(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<Option<Vec<DeploymentSpec>>, LookupError> {
        if self.catalog.model_names.contains(model) {
            return Ok(None);
        }
        if let Some(team) = team_id {
            let owned = get_all_deployments(self.catalog, model, None, Some(team));
            if !owned.is_empty() {
                return Ok(Some(owned));
            }
        }
        let provider = (self.inferred_provider)(model);
        let mut pool = pattern_pool(
            self.catalog,
            &self.catalog.pattern_router,
            model,
            provider.as_deref(),
        )?;
        if pool.is_empty()
            && let Some(patterns) =
                team_id.and_then(|team| self.catalog.team_pattern_routers.get(team))
        {
            pool = pattern_pool(self.catalog, &patterns.router, model, provider.as_deref())?;
        }
        Ok(Some(pool).filter(|found| !found.is_empty()))
    }

    // Router.resolve_model_name_from_model_id: the public name, then the id, then a provider model that ends in the id.
    pub fn resolve_model_name_from_model_id(
        &self,
        model_id: Option<&str>,
    ) -> Result<Option<String>, LookupError> {
        let Some(model_id) = model_id.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        if self.catalog.model_names.contains(model_id) {
            return Ok(Some(model_id.to_owned()));
        }
        if self.has_model_id(model_id) {
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
}

// The provider-model arm of resolve_model_name_from_model_id: the exact value, a '/id' or a ':id' suffix, case kept.
fn provider_model_matches(actual: &str, model_id: &str) -> bool {
    actual == model_id
        || actual.ends_with(&format!("/{model_id}"))
        || actual.ends_with(&format!(":{model_id}"))
}
