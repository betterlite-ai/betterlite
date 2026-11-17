pub mod cost_routing;
pub mod lar1_routing;
pub mod latency_routing;
pub mod least_busy;
pub mod usage_routing;
pub mod usage_routing_v2;

use rand::Rng;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::candidates::weights;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;
use crate::simple_shuffle::pick;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::Resolution;

// The selection algorithm a configured routing strategy runs, once B395 has resolved which one applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    SimpleShuffle,
    LeastBusy,
    LatencyBasedRouting,
    CostBasedRouting,
    UsageBasedRouting,
    UsageBasedRoutingV2,
    Lar1,
}

impl Strategy {
    // provider-budget-routing selects no deployment of its own, so LiteLLM falls through to the no-deployment error
    #[must_use]
    pub const fn of(routing: RoutingStrategy) -> Option<Self> {
        match routing {
            RoutingStrategy::SimpleShuffle => Some(Self::SimpleShuffle),
            RoutingStrategy::LeastBusy => Some(Self::LeastBusy),
            RoutingStrategy::LatencyBasedRouting => Some(Self::LatencyBasedRouting),
            RoutingStrategy::CostBasedRouting => Some(Self::CostBasedRouting),
            RoutingStrategy::UsageBasedRouting => Some(Self::UsageBasedRouting),
            RoutingStrategy::UsageBasedRoutingV2 => Some(Self::UsageBasedRoutingV2),
            RoutingStrategy::Lar1 => Some(Self::Lar1),
            RoutingStrategy::ProviderBudgetRouting => None,
        }
    }
}

// What one strategy weighs the pool by, besides the randomness the caller injects.
pub struct StrategyContext<'context> {
    pub deployments: &'context [DeploymentSpec],
    pub route: &'context RouteContext<'context>,
    pub dispatch: &'context mut StrategyDispatch,
    pub resolution: &'context Resolution,
}

// The one selection call a route makes: each arm calls only the code that owns that strategy.
pub async fn select<R>(
    strategy: Strategy,
    context: StrategyContext<'_>,
    random: &mut R,
) -> Result<usize, RoutingError>
where
    R: Rng + Send,
{
    match strategy {
        Strategy::SimpleShuffle => Ok(pick(&weights(context.deployments)?, random)?),
        Strategy::LeastBusy => least_busy::select(&context, random).await,
        Strategy::LatencyBasedRouting => latency_routing::select(&context, random).await,
        Strategy::CostBasedRouting => cost_routing::select(&context, random).await,
        Strategy::UsageBasedRouting => usage_routing::select(&context, random).await,
        Strategy::UsageBasedRoutingV2 => usage_routing_v2::select(&context, random).await,
        Strategy::Lar1 => lar1_routing::select(&context, random).await,
    }
}

// A strategy no task has built yet: every strategy module starts from this and replaces it when its selection lands.
macro_rules! strategy_stub {
    ($entry:literal) => {
        use std::future::{Ready, ready};

        use rand::Rng;

        use crate::router_core::error::RoutingError;
        use crate::strategy::StrategyContext;

        pub(super) fn select<R>(
            _context: &StrategyContext<'_>,
            _random: &mut R,
        ) -> Ready<Result<usize, RoutingError>>
        where
            R: Rng,
        {
            ready(Err(RoutingError::NotBuilt { entry: $entry }))
        }
    };
}
pub(crate) use strategy_stub;
