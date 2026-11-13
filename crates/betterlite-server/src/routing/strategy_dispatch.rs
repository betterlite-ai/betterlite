use betterlite_config::types::RoutingStrategy as ConfiguredStrategy;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;

#[must_use]
pub const fn strategy_from_config(strategy: Option<ConfiguredStrategy>) -> RoutingStrategy {
    match strategy {
        None | Some(ConfiguredStrategy::SimpleShuffle) => RoutingStrategy::SimpleShuffle,
        Some(ConfiguredStrategy::LeastBusy) => RoutingStrategy::LeastBusy,
        Some(ConfiguredStrategy::UsageBasedRouting) => RoutingStrategy::UsageBasedRouting,
        Some(ConfiguredStrategy::LatencyBasedRouting) => RoutingStrategy::LatencyBasedRouting,
        Some(ConfiguredStrategy::CostBasedRouting) => RoutingStrategy::CostBasedRouting,
        Some(ConfiguredStrategy::UsageBasedRoutingV2) => RoutingStrategy::UsageBasedRoutingV2,
        Some(ConfiguredStrategy::Lar1) => RoutingStrategy::Lar1,
    }
}

mod tests;
