use std::collections::{BTreeMap, BTreeSet};
use std::mem::take;

use arc_swap::ArcSwap;
use betterlite_models::decisions::Json;

use crate::deployment_identity::construct::DeploymentInput;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::wildcard_catalog::error::PatternCatalogError;
use crate::wildcard_catalog::{Candidate, PatternCatalog};

// A team's wildcard router and the ids it holds, since PatternCatalog does not say when it is empty.
#[derive(Clone, Debug, Default)]
pub struct TeamPatterns {
    pub router: PatternCatalog,
    pub model_ids: Vec<Option<String>>,
}

// One immutable view of the deployments and the indices LiteLLM keeps beside the model list.
#[derive(Clone, Debug, Default)]
pub struct DeploymentCatalog {
    pub models: Vec<DeploymentSpec>,
    pub model_names: BTreeSet<String>,
    pub deployment_names: Vec<String>,
    pub provider_default_ids: Vec<String>,
    // the entries LiteLLM queues for a retry once their provider resolves
    pub provider_unresolved: Vec<DeploymentInput>,
    pub pattern_router: PatternCatalog,
    pub team_pattern_routers: BTreeMap<String, TeamPatterns>,
    // the last deployment pushed under an id wins, and removing that id drops the entry even when a twin remains
    pub id_index: BTreeMap<String, usize>,
    pub name_index: BTreeMap<String, Vec<usize>>,
    pub team_index: BTreeMap<(String, String), Vec<usize>>,
}

fn info_text<'spec>(spec: &'spec DeploymentSpec, key: &str) -> Option<&'spec str> {
    match spec.model_info.get(key) {
        Some(Json::Text(value)) if !value.is_empty() => Some(value),
        _ => None,
    }
}

fn shifted(indices: &[usize], removed: usize) -> Vec<usize> {
    indices
        .iter()
        .filter(|index| **index != removed)
        .map(|index| index.saturating_sub(usize::from(index.cmp(&removed).is_gt())))
        .collect()
}

impl DeploymentCatalog {
    #[must_use]
    pub fn team_public_model_names(&self) -> BTreeSet<&str> {
        self.team_index
            .keys()
            .map(|(_, public_name)| public_name.as_str())
            .collect()
    }

    // The pattern half of _add_deployment: the name list, then the wildcard routers.
    pub fn index_patterns(&mut self, spec: &DeploymentSpec) -> Result<(), PatternCatalogError> {
        let model = match spec.litellm_params.get("model") {
            Some(Json::Text(model)) => model.clone(),
            _ => String::new(),
        };
        self.deployment_names.push(model.clone());
        let candidate = Candidate {
            model_id: spec.id().map(str::to_owned),
            model,
        };
        if spec.model_name.contains('*') {
            self.pattern_router
                .add_pattern(&spec.model_name, candidate.clone())?;
            if let Some(id) = spec.id().filter(|id| !id.is_empty()) {
                self.provider_default_ids.push(id.to_owned());
            }
        }
        if let (Some(team), Some(public_name)) = (
            info_text(spec, "team_id"),
            info_text(spec, "team_public_model_name").filter(|name| name.contains('*')),
        ) {
            let patterns = self
                .team_pattern_routers
                .entry(team.to_owned())
                .or_default();
            patterns.router.add_pattern(public_name, candidate)?;
            patterns.model_ids.push(spec.id().map(str::to_owned));
        }
        Ok(())
    }

    // _add_model_to_list_and_index_map
    pub fn push(&mut self, spec: DeploymentSpec) {
        let index = self.models.len();
        if let Some(id) = spec.id() {
            self.id_index.insert(id.to_owned(), index);
        }
        if !spec.model_name.is_empty() {
            self.name_index
                .entry(spec.model_name.clone())
                .or_default()
                .push(index);
        }
        if let (Some(team), Some(public_name)) = (
            info_text(&spec, "team_id"),
            info_text(&spec, "team_public_model_name"),
        ) {
            self.team_index
                .entry((team.to_owned(), public_name.to_owned()))
                .or_default()
                .push(index);
        }
        self.models.push(spec);
    }

    // The pop and _update_deployment_indices_after_removal: the deployment at index goes and every later index moves down.
    pub fn remove(&mut self, model_id: &str, index: usize) -> Option<DeploymentSpec> {
        self.models.get(index)?;
        let removed = self.models.remove(index);
        for position in self.id_index.values_mut() {
            *position = position.saturating_sub(usize::from((*position).cmp(&index).is_gt()));
        }
        self.id_index.remove(model_id);
        for (name, indices) in take(&mut self.name_index) {
            let kept = shifted(&indices, index);
            if kept.is_empty() {
                self.model_names.remove(&name);
            } else {
                self.name_index.insert(name, kept);
            }
        }
        self.team_index = take(&mut self.team_index)
            .into_iter()
            .map(|(key, indices)| (key, shifted(&indices, index)))
            .filter(|(_, indices)| !indices.is_empty())
            .collect();
        self.pattern_router.remove_deployment(model_id);
        for patterns in self.team_pattern_routers.values_mut() {
            patterns.router.remove_deployment(model_id);
            patterns
                .model_ids
                .retain(|id| id.as_deref() != Some(model_id));
        }
        self.team_pattern_routers
            .retain(|_, patterns| !patterns.model_ids.is_empty());
        Some(removed)
    }
}

// The published catalog; readers keep the Arc they took while a writer builds and swaps in the next one.
pub type CatalogHandle = ArcSwap<DeploymentCatalog>;
