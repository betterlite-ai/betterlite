use crate::strategy_dispatch::error::StrategyDispatchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoutingStrategy {
    SimpleShuffle,
    Lar1,
    LeastBusy,
    LatencyBasedRouting,
    CostBasedRouting,
    UsageBasedRoutingV2,
    UsageBasedRouting,
    ProviderBudgetRouting,
}

pub const VALID_ROUTING_STRATEGIES: [RoutingStrategy; 8] = [
    RoutingStrategy::SimpleShuffle,
    RoutingStrategy::Lar1,
    RoutingStrategy::LeastBusy,
    RoutingStrategy::LatencyBasedRouting,
    RoutingStrategy::CostBasedRouting,
    RoutingStrategy::UsageBasedRoutingV2,
    RoutingStrategy::UsageBasedRouting,
    RoutingStrategy::ProviderBudgetRouting,
];

// LiteLLM prints this list in its ValueError, in this order and with these quotes
pub(crate) const VALID_STRATEGY_LIST: &str = "['simple-shuffle', 'lar1', 'least-busy', 'latency-based-routing', 'cost-based-routing', 'usage-based-routing-v2', 'usage-based-routing', 'provider-budget-routing']";

impl RoutingStrategy {
    pub fn parse(value: &str) -> Result<Self, StrategyDispatchError> {
        VALID_ROUTING_STRATEGIES
            .iter()
            .copied()
            .find(|strategy| strategy.wire_name() == value)
            .ok_or_else(|| StrategyDispatchError::InvalidRoutingStrategy {
                value: value.to_owned(),
                valid: VALID_STRATEGY_LIST,
            })
    }

    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::SimpleShuffle => "simple-shuffle",
            Self::Lar1 => "lar1",
            Self::LeastBusy => "least-busy",
            Self::LatencyBasedRouting => "latency-based-routing",
            Self::CostBasedRouting => "cost-based-routing",
            Self::UsageBasedRoutingV2 => "usage-based-routing-v2",
            Self::UsageBasedRouting => "usage-based-routing",
            Self::ProviderBudgetRouting => "provider-budget-routing",
        }
    }

    #[must_use]
    pub const fn has_selector(self) -> bool {
        match self {
            Self::LeastBusy
            | Self::LatencyBasedRouting
            | Self::CostBasedRouting
            | Self::UsageBasedRoutingV2
            | Self::UsageBasedRouting => true,
            Self::SimpleShuffle | Self::Lar1 | Self::ProviderBudgetRouting => false,
        }
    }

    #[must_use]
    pub const fn is_overridable(self) -> bool {
        match self {
            Self::SimpleShuffle
            | Self::LeastBusy
            | Self::LatencyBasedRouting
            | Self::CostBasedRouting
            | Self::UsageBasedRoutingV2
            | Self::UsageBasedRouting => true,
            Self::Lar1 | Self::ProviderBudgetRouting => false,
        }
    }
}
