use std::collections::{BTreeMap, BTreeSet};

use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::strategy::RoutingStrategy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStrategy {
    Priority,
    Routing(RoutingStrategy),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingGroupRoute {
    pub name: String,
    pub models: Vec<String>,
    pub strategy: GroupStrategy,
    pub selector: Option<SelectorId>,
}

#[derive(Debug, Clone, Default)]
pub struct RoutingGroups {
    routes: BTreeMap<String, RoutingGroupRoute>,
    model_to_group: BTreeMap<String, String>,
}

impl RoutingGroups {
    pub fn new(routes: Vec<RoutingGroupRoute>) -> Result<Self, StrategyDispatchError> {
        let mut groups = Self::default();
        for route in routes {
            if groups.routes.contains_key(&route.name) {
                return Err(StrategyDispatchError::DuplicateGroup { group: route.name });
            }
            groups.claim_models(&route)?;
            groups.routes.insert(route.name.clone(), route);
        }
        Ok(groups)
    }

    #[must_use]
    pub fn route(&self, model: &str) -> Option<&RoutingGroupRoute> {
        if let Some(route) = self.routes.get(model) {
            return Some(route);
        }
        let group = self.model_to_group.get(model)?;
        self.routes.get(group)
    }

    #[must_use]
    pub fn globally_registered(&self) -> BTreeSet<RoutingStrategy> {
        let mut strategies = BTreeSet::new();
        for route in self.routes.values() {
            if let GroupStrategy::Routing(strategy) = route.strategy {
                strategies.insert(strategy);
            }
        }
        strategies
    }

    fn claim_models(&mut self, route: &RoutingGroupRoute) -> Result<(), StrategyDispatchError> {
        if !matches!(route.strategy, GroupStrategy::Routing(_)) {
            return Ok(());
        }
        for model in &route.models {
            if let Some(first) = self
                .model_to_group
                .insert(model.clone(), route.name.clone())
            {
                return Err(StrategyDispatchError::DuplicateGroupModel {
                    model: model.clone(),
                    first,
                    second: route.name.clone(),
                });
            }
        }
        Ok(())
    }
}
