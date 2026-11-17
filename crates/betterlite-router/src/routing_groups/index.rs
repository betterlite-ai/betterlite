use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_groups::error::RoutingGroupError;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::priority::{group_row, prioritized};
use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroupRoute, RoutingGroups};
use crate::strategy_dispatch::service::StrategyDispatch;

// Router._routing_groups plus the names that shadow a same-named group.
#[derive(Clone, Debug, Default)]
pub struct RoutingGroupIndex {
    groups: IndexMap<String, RoutingGroup>,
    shadowing: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
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
        aliases: &BTreeMap<String, String>,
    ) -> Result<Built, RoutingGroupError> {
        let mut index = Self {
            groups: IndexMap::new(),
            shadowing: aliases.keys().chain(known_model_names).cloned().collect(),
            aliases: aliases.clone(),
        };
        let mut warnings = Vec::new();
        for group in groups {
            if index.groups.contains_key(&group.group_name) {
                return Err(RoutingGroupError::DuplicateName {
                    group: group.group_name,
                });
            }
            let shadowed = index.shadowing.contains(&group.group_name);
            if shadowed && matches!(group.routing_strategy, GroupStrategy::Priority) {
                return Err(RoutingGroupError::PriorityShadow);
            }
            if shadowed {
                warnings.push(format!(
                    "routing_groups: group_name '{}' is shadowed by an existing model_name or model_group_alias; the group's strategy still applies to its members, but the name is not callable until renamed.",
                    group.group_name
                ));
            }
            index.groups.insert(group.group_name.clone(), group);
        }
        Ok(Built { index, warnings })
    }

    // get_routing_group: a deployment name or alias added after init shadows a same-named group.
    #[must_use]
    pub fn callable_group(&self, name: &str) -> Option<&RoutingGroup> {
        if self.shadowing.contains(name) {
            return None;
        }
        self.groups.get(name)
    }

    // _is_priority_routing_group: _get_model_from_alias names the group, the alias does not.
    #[must_use]
    pub fn is_priority_group(&self, name: &str) -> bool {
        let resolved = self.aliases.get(name).map_or(name, String::as_str);
        self.callable_group(resolved)
            .is_some_and(|group| matches!(group.routing_strategy, GroupStrategy::Priority))
    }

    // _replace_routing_groups: each group's own selector with the args its owner decoded, then B019's routes over the same names and members.
    pub fn dispatch_groups(
        &self,
        dispatch: &mut StrategyDispatch,
        arguments_of: &dyn Fn(&RoutingGroup) -> Option<SelectorArgs>,
    ) -> Result<RoutingGroups, RoutingGroupError> {
        let mut routes = Vec::with_capacity(self.groups.len());
        for group in self.groups.values() {
            let selector = match group.routing_strategy {
                GroupStrategy::Priority => None,
                GroupStrategy::Routing(strategy) => {
                    dispatch.build_group_selector(strategy, arguments_of(group))?
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

    // routing_group_has_alternatives: the alias target names the group, and each member's deployment count.
    #[must_use]
    pub fn has_alternatives(&self, name: &str, deployments_of: &dyn Fn(&str) -> usize) -> bool {
        let resolved = self.aliases.get(name).map_or(name, String::as_str);
        let Some(group) = self.callable_group(resolved) else {
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
        let groups: Vec<&RoutingGroup> = requested.map_or_else(
            || {
                self.groups
                    .keys()
                    .filter_map(|name| self.callable_group(name))
                    .collect()
            },
            |name| self.callable_group(name).into_iter().collect(),
        );
        groups
            .into_iter()
            .flat_map(|group| group_rows(group, deployments_of))
            .collect()
    }
}

fn group_rows(
    group: &RoutingGroup,
    deployments_of: &dyn Fn(&str) -> Vec<DeploymentSpec>,
) -> Vec<DeploymentSpec> {
    let mut rows = Vec::new();
    for member in &group.models {
        for deployment in deployments_of(member) {
            rows.push(group_row(
                &group.group_name,
                &prioritized(group, &deployment),
            ));
        }
    }
    rows
}
