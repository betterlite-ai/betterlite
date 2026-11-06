#![cfg(test)]

mod strategy_dispatch_behaviors;
mod strategy_dispatch_boundaries;

use std::collections::BTreeSet;

use crate::simple_shuffle::Candidate;
use crate::strategy_dispatch::args::{LatencyArgs, SelectorArgs};
use crate::strategy_dispatch::groups::{
    GroupStrategy, RoutingGroupRoute, RoutingGroups, RoutingScope,
};
use crate::strategy_dispatch::selector::SelectorState;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{RoutingRequest, StrategyOverride};

const SEED: u64 = 7;

fn new_dispatch(strategy: RoutingStrategy) -> StrategyDispatch {
    StrategyDispatch::new(strategy, None).unwrap()
}

fn callback_strategies(state: &SelectorState) -> Vec<RoutingStrategy> {
    state
        .callbacks
        .iter()
        .map(|selector| selector.strategy)
        .collect()
}

fn input_callback_strategies(state: &SelectorState) -> Vec<RoutingStrategy> {
    state
        .input_callbacks
        .iter()
        .map(|selector| selector.strategy)
        .collect()
}

fn request_for(model: &'static str) -> RoutingRequest<'static> {
    RoutingRequest {
        resolved_model: model,
        strategy: StrategyOverride::Absent,
        bindings: None,
    }
}

fn cheap_groups() -> RoutingGroups {
    RoutingGroups::new(vec![
        RoutingGroupRoute {
            name: "cheap".to_owned(),
            models: vec!["fast".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::LeastBusy),
            selector: None,
        },
        RoutingGroupRoute {
            name: "prio".to_owned(),
            models: vec!["fast".to_owned(), "slow".to_owned()],
            strategy: GroupStrategy::Priority,
            selector: None,
        },
    ])
    .unwrap()
}

fn no_shadowing() -> BTreeSet<String> {
    BTreeSet::new()
}

fn scope<'catalog>(
    groups: &'catalog RoutingGroups,
    shadowing: &'catalog BTreeSet<String>,
) -> RoutingScope<'catalog> {
    RoutingScope { groups, shadowing }
}

fn latency_arguments(ttft_percentile: Option<f64>) -> SelectorArgs {
    SelectorArgs::Latency(LatencyArgs {
        ttl_seconds: 3600.0,
        lowest_latency_buffer: 0.0,
        max_latency_list_size: 10,
        ttft_percentile,
    })
}

fn candidates() -> [Candidate; 1] {
    [Candidate {
        weight: 0,
        rpm: 0,
        tpm: 0,
    }]
}
