#![cfg(test)]

use betterlite_config::types::{RouterSettings, RoutingStrategy as ConfiguredStrategy};
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use rstest::rstest;

use super::strategy_from_config;

#[rstest]
#[case::absent(None, RoutingStrategy::SimpleShuffle)]
#[case::simple_shuffle(
    Some(ConfiguredStrategy::SimpleShuffle),
    RoutingStrategy::SimpleShuffle
)]
#[case::least_busy(Some(ConfiguredStrategy::LeastBusy), RoutingStrategy::LeastBusy)]
#[case::usage_based(
    Some(ConfiguredStrategy::UsageBasedRouting),
    RoutingStrategy::UsageBasedRouting
)]
#[case::latency(
    Some(ConfiguredStrategy::LatencyBasedRouting),
    RoutingStrategy::LatencyBasedRouting
)]
#[case::cost(
    Some(ConfiguredStrategy::CostBasedRouting),
    RoutingStrategy::CostBasedRouting
)]
#[case::usage_based_v2(
    Some(ConfiguredStrategy::UsageBasedRoutingV2),
    RoutingStrategy::UsageBasedRoutingV2
)]
#[case::lar1(Some(ConfiguredStrategy::Lar1), RoutingStrategy::Lar1)]
fn maps_every_configured_strategy(
    #[case] configured: Option<ConfiguredStrategy>,
    #[case] expected: RoutingStrategy,
) {
    assert_eq!(strategy_from_config(configured), expected);
}

#[test]
fn absent_configuration_builds_a_simple_shuffle_dispatch() {
    let strategy = strategy_from_config(RouterSettings::default().routing_strategy);
    let dispatch = StrategyDispatch::new(strategy, None).unwrap();
    let state = dispatch.state();
    assert!(state.selector.is_none());
    assert!(state.callbacks.is_empty());
    assert!(state.input_callbacks.is_empty());
}
