use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::strategy::RoutingStrategy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageArgs {
    pub ttl_seconds: i64,
}

impl Default for UsageArgs {
    fn default() -> Self {
        Self { ttl_seconds: 60 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyArgs {
    pub ttl_seconds: f64,
    pub lowest_latency_buffer: f64,
    pub max_latency_list_size: u32,
    pub ttft_percentile: Option<f64>,
}

impl Default for LatencyArgs {
    fn default() -> Self {
        Self {
            ttl_seconds: 3600.0,
            lowest_latency_buffer: 0.0,
            max_latency_list_size: 10,
            ttft_percentile: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectorArgs {
    Fixed,
    Usage(UsageArgs),
    Latency(LatencyArgs),
}

impl SelectorArgs {
    pub fn for_strategy(
        strategy: RoutingStrategy,
        supplied: Option<Self>,
    ) -> Result<Self, StrategyDispatchError> {
        match strategy {
            RoutingStrategy::UsageBasedRouting | RoutingStrategy::UsageBasedRoutingV2 => {
                match supplied {
                    None => Ok(Self::Usage(UsageArgs::default())),
                    Some(arguments @ Self::Usage(_)) => Ok(arguments),
                    Some(arguments) => Err(StrategyDispatchError::ArgsMismatch {
                        strategy,
                        expected: "usage-based arguments",
                        supplied: arguments.kind_name(),
                    }),
                }
            }
            RoutingStrategy::LatencyBasedRouting => {
                let arguments = match supplied {
                    None => Self::Latency(LatencyArgs::default()),
                    Some(arguments @ Self::Latency(_)) => arguments,
                    Some(arguments) => {
                        return Err(StrategyDispatchError::ArgsMismatch {
                            strategy,
                            expected: "latency-based arguments",
                            supplied: arguments.kind_name(),
                        });
                    }
                };
                if let Self::Latency(latency) = arguments
                    && let Some(percentile) = latency.ttft_percentile
                    && !(percentile > 0.0 && percentile <= 1.0)
                {
                    return Err(StrategyDispatchError::InvalidTtftPercentile { value: percentile });
                }
                Ok(arguments)
            }
            RoutingStrategy::SimpleShuffle
            | RoutingStrategy::Lar1
            | RoutingStrategy::LeastBusy
            | RoutingStrategy::CostBasedRouting
            | RoutingStrategy::ProviderBudgetRouting => Ok(Self::Fixed),
        }
    }

    #[must_use]
    pub fn defaults_for(strategy: RoutingStrategy) -> Self {
        match strategy {
            RoutingStrategy::UsageBasedRouting | RoutingStrategy::UsageBasedRoutingV2 => {
                Self::Usage(UsageArgs::default())
            }
            RoutingStrategy::LatencyBasedRouting => Self::Latency(LatencyArgs::default()),
            RoutingStrategy::SimpleShuffle
            | RoutingStrategy::Lar1
            | RoutingStrategy::LeastBusy
            | RoutingStrategy::CostBasedRouting
            | RoutingStrategy::ProviderBudgetRouting => Self::Fixed,
        }
    }

    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        match self {
            Self::Fixed => "no routing arguments",
            Self::Usage(_) => "usage-based arguments",
            Self::Latency(_) => "latency-based arguments",
        }
    }
}
