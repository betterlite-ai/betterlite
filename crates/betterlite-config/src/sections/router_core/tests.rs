#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::RoutingStrategy;

fn loaded(source: &str) -> RoutingStrategy {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .router_core
    .routing_strategy
}

fn failed(source: &str) -> ConfigError {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err()
}

#[rstest]
#[case::absent("router_settings: {}\n", RoutingStrategy::SimpleShuffle)]
#[case::simple_shuffle(
    "router_settings:\n  routing_strategy: simple-shuffle\n",
    RoutingStrategy::SimpleShuffle
)]
#[case::least_busy(
    "router_settings:\n  routing_strategy: least-busy\n",
    RoutingStrategy::LeastBusy
)]
#[case::latency(
    "router_settings:\n  routing_strategy: latency-based-routing\n",
    RoutingStrategy::LatencyBasedRouting
)]
#[case::cost(
    "router_settings:\n  routing_strategy: cost-based-routing\n",
    RoutingStrategy::CostBasedRouting
)]
#[case::usage(
    "router_settings:\n  routing_strategy: usage-based-routing\n",
    RoutingStrategy::UsageBasedRouting
)]
#[case::usage_v2(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n",
    RoutingStrategy::UsageBasedRoutingV2
)]
#[case::lar1("router_settings:\n  routing_strategy: lar1\n", RoutingStrategy::Lar1)]
fn router_core_routing_strategy_loads(#[case] source: &str, #[case] expected: RoutingStrategy) {
    assert_eq!(loaded(source), expected);
}

#[test]
fn router_core_defaults_to_simple_shuffle() {
    assert_eq!(loaded("model_list: []\n"), RoutingStrategy::SimpleShuffle);
}

#[test]
fn router_core_rejects_an_unknown_strategy() {
    assert!(matches!(
        failed("router_settings:\n  routing_strategy: fastest\n"),
        ConfigError::Yaml { .. }
    ));
}
