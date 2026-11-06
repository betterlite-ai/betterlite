use rand::Rng;

use crate::simple_shuffle::{Candidate, pick};
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::groups::RoutingGroups;
use crate::strategy_dispatch::override_resolution::resolve;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, RoutingRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorCall {
    LeastBusy,
    UsageBasedV1,
    UsageBasedV2,
    CostBased,
    LatencyBased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    SimpleShuffle {
        index: usize,
    },
    Selector {
        selector: SelectorId,
        call: SelectorCall,
    },
    NoSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOutcome {
    pub resolution: Resolution,
    pub selection: Selection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadPrefetchPlan<'deployments, Deployment> {
    pub usage_selector: Option<SelectorId>,
    pub deployments: &'deployments [Deployment],
}

pub fn select<R>(
    dispatch: &mut StrategyDispatch,
    request: &mut RoutingRequest<'_>,
    groups: &RoutingGroups,
    candidates: &[Candidate],
    random: &mut R,
) -> Result<SelectionOutcome, StrategyDispatchError>
where
    R: Rng,
{
    let resolution = resolve(dispatch, request, groups);
    let selection = match resolution.strategy {
        RoutingStrategy::SimpleShuffle => Selection::SimpleShuffle {
            index: pick(candidates, random)?,
        },
        RoutingStrategy::LeastBusy
        | RoutingStrategy::LatencyBasedRouting
        | RoutingStrategy::CostBasedRouting
        | RoutingStrategy::UsageBasedRoutingV2
        | RoutingStrategy::UsageBasedRouting => {
            match (
                selector_call(resolution.strategy, resolution.selector),
                resolution.selector,
            ) {
                (Some(call), Some(selector)) => Selection::Selector { selector, call },
                _ => Selection::NoSelector,
            }
        }
        RoutingStrategy::Lar1 | RoutingStrategy::ProviderBudgetRouting => Selection::NoSelector,
    };
    Ok(SelectionOutcome {
        resolution,
        selection,
    })
}

pub fn read_prefetch_plan<'deployments, Deployment>(
    dispatch: &mut StrategyDispatch,
    request: &mut RoutingRequest<'_>,
    groups: &RoutingGroups,
    deployments: &'deployments [Deployment],
) -> Option<ReadPrefetchPlan<'deployments, Deployment>> {
    let resolution = resolve(dispatch, request, groups);
    if deployments.is_empty() {
        return None;
    }
    let usage_selector = if resolution.strategy == RoutingStrategy::UsageBasedRoutingV2
        && let Some(selector) = resolution
            .selector
            .and_then(|id| dispatch.selectors.get(id.0))
        && selector.strategy == RoutingStrategy::UsageBasedRoutingV2
    {
        resolution.selector
    } else {
        None
    };
    Some(ReadPrefetchPlan {
        usage_selector,
        deployments,
    })
}

#[must_use]
pub const fn selector_call(
    strategy: RoutingStrategy,
    selector: Option<SelectorId>,
) -> Option<SelectorCall> {
    match selector {
        None => None,
        Some(_) => match strategy {
            RoutingStrategy::LeastBusy => Some(SelectorCall::LeastBusy),
            RoutingStrategy::UsageBasedRouting => Some(SelectorCall::UsageBasedV1),
            RoutingStrategy::UsageBasedRoutingV2 => Some(SelectorCall::UsageBasedV2),
            RoutingStrategy::CostBasedRouting => Some(SelectorCall::CostBased),
            RoutingStrategy::LatencyBasedRouting => Some(SelectorCall::LatencyBased),
            RoutingStrategy::SimpleShuffle
            | RoutingStrategy::Lar1
            | RoutingStrategy::ProviderBudgetRouting => None,
        },
    }
}
