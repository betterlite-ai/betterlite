use std::collections::BTreeSet;

use crate::benchmark_aggregates::group::{BenchmarkGroup, benchmark_group, summed_agg_row};
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::router_dependencies::{StrategyRouterKind, classify_strategy_router_model};

// The parts of a Router deployment the idle-group list reads, already narrowed to strings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyDeployment<'a> {
    pub model_name: Option<&'a str>,
    pub model: Option<&'a str>,
}

const fn router_type_name(kind: StrategyRouterKind) -> &'static str {
    match kind {
        StrategyRouterKind::Semantic => "semantic",
        StrategyRouterKind::Complexity => "complexity",
        StrategyRouterKind::Adaptive => "adaptive",
        StrategyRouterKind::Quality => "quality",
    }
}

// LiteLLM's _strategy_router_key: a named deployment whose model is a non-semantic strategy router
#[must_use]
pub fn strategy_router_key(deployment: StrategyDeployment<'_>) -> Option<(String, String)> {
    let model_name = deployment.model_name.filter(|name| !name.is_empty())?;
    let kind = classify_strategy_router_model(deployment.model?)?;
    if kind == StrategyRouterKind::Semantic {
        return None;
    }
    Some((model_name.to_owned(), router_type_name(kind).to_owned()))
}

// LiteLLM's _idle_router_groups: a zeroed group for every configured strategy router not covered
#[must_use]
pub fn idle_router_groups(
    deployments: Option<&[StrategyDeployment<'_>]>,
    covered: &BTreeSet<(String, String)>,
) -> Vec<BenchmarkGroup> {
    let Some(deployments) = deployments else {
        return Vec::new();
    };
    let idle: BTreeSet<(String, String)> = deployments
        .iter()
        .filter_map(|deployment| strategy_router_key(*deployment))
        .filter(|key| !covered.contains(key))
        .collect();
    let zero = summed_agg_row(&[]);
    let mut groups = Vec::with_capacity(idle.len());
    for (router_name, router_type) in idle {
        let row = SessionAggRow {
            router_name,
            router_type,
            ..zero.clone()
        };
        groups.push(benchmark_group(&row));
    }
    groups
}
