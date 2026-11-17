use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_groups::error::RoutingGroupError;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::priority::{group_row, prioritized};
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroupRoute, RoutingGroups};
use crate::strategy_dispatch::service::StrategyDispatch;

// Router._routing_groups plus the names that shadow a same-named group.
#[derive(Clone, Debug, Default)]
pub struct RoutingGroupIndex {
    groups: IndexMap<String, RoutingGroup>,
    model_to_group: BTreeMap<String, String>,
    shadowing: BTreeSet<String>,
}

// The index plus the shadowing warnings _init_routing_groups logs.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub index: RoutingGroupIndex,
    pub warnings: Vec<String>,
}

impl RoutingGroupIndex {
    // _init_routing_groups: a priority group may not be shadowed, a strategy group only warns.
    pub fn build(
        groups: Vec<RoutingGroup>,
        known_model_names: &BTreeSet<String>,
        alias_names: &BTreeSet<String>,
    ) -> Result<Built, RoutingGroupError> {
        let mut index = Self {
            shadowing: known_model_names.union(alias_names).cloned().collect(),
            ..Self::default()
        };
        let mut warnings = Vec::new();
        for group in groups {
            if index.groups.contains_key(&group.group_name) {
                return Err(RoutingGroupError::DuplicateName {
                    group: group.group_name,
                });
            }
            if index.shadowing.contains(&group.group_name) {
                if matches!(group.routing_strategy, GroupStrategy::Priority) {
                    return Err(RoutingGroupError::PriorityShadow);
                }
                warnings.push(format!(
                    "routing_groups: group_name '{}' is shadowed by an existing model_name or model_group_alias; the group's strategy still applies to its members, but the name is not callable until renamed.",
                    group.group_name
                ));
            }
            if !matches!(group.routing_strategy, GroupStrategy::Priority) {
                for model in &group.models {
                    index
                        .model_to_group
                        .insert(model.clone(), group.group_name.clone());
                }
            }
            index.groups.insert(group.group_name.clone(), group);
        }
        Ok(Built { index, warnings })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    // get_routing_group: a deployment name or alias added after init shadows a same-named group.
    #[must_use]
    pub fn callable_group(&self, name: &str) -> Option<&RoutingGroup> {
        if self.shadowing.contains(name) {
            return None;
        }
        self.groups.get(name)
    }

    // _is_priority_routing_group, over the alias-resolved name the caller passes.
    #[must_use]
    pub fn is_priority_group(&self, name: &str) -> bool {
        self.callable_group(name)
            .is_some_and(|group| matches!(group.routing_strategy, GroupStrategy::Priority))
    }

    // _model_to_group: the group a member model belongs to for strategy dispatch.
    #[must_use]
    pub fn claiming_group(&self, model: &str) -> Option<&RoutingGroup> {
        self.model_to_group
            .get(model)
            .and_then(|group| self.groups.get(group))
    }

    #[must_use]
    pub fn group_names(&self) -> impl Iterator<Item = &str> {
        self.groups.keys().map(String::as_str)
    }

    // _replace_routing_groups: each group's own selector, then B019's routes over the same names and members.
    pub fn dispatch_groups(
        &self,
        dispatch: &mut StrategyDispatch,
    ) -> Result<RoutingGroups, RoutingGroupError> {
        let mut routes = Vec::with_capacity(self.groups.len());
        for group in self.groups.values() {
            let selector = match group.routing_strategy {
                GroupStrategy::Priority => None,
                GroupStrategy::Routing(strategy) => {
                    dispatch.build_group_selector(strategy, None)?
                }
            };
            routes.push(RoutingGroupRoute {
                name: group.group_name.clone(),
                models: group.models.clone(),
                strategy: group.routing_strategy,
                selector,
            });
        }
        Ok(RoutingGroups::new(routes)?)
    }

    // routing_group_has_alternatives, over the alias-resolved name and each member's deployment count.
    #[must_use]
    pub fn has_alternatives(&self, name: &str, deployments_of: &dyn Fn(&str) -> usize) -> bool {
        let Some(group) = self.callable_group(name) else {
            return false;
        };
        group
            .models
            .iter()
            .map(|member| deployments_of(member))
            .sum::<usize>()
            > 1
    }

    // get_model_list_from_routing_groups: each member deployment re-emitted under its group's name.
    #[must_use]
    pub fn rows(
        &self,
        requested: Option<&str>,
        deployments_of: &dyn Fn(&str) -> Vec<DeploymentSpec>,
    ) -> Vec<DeploymentSpec> {
        let groups: Vec<&RoutingGroup> = match requested {
            Some(name) => self.callable_group(name).into_iter().collect(),
            None => self
                .group_names()
                .filter_map(|name| self.callable_group(name))
                .collect(),
        };
        let mut rows = Vec::new();
        for group in groups {
            for member in &group.models {
                for deployment in deployments_of(member) {
                    rows.push(group_row(
                        &group.group_name,
                        &prioritized(group, &deployment),
                    ));
                }
            }
        }
        rows
    }
}
