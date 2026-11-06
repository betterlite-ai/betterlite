use std::collections::BTreeSet;

use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroups};
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::{RoutingStrategy, VALID_ROUTING_STRATEGIES};
use crate::strategy_dispatch::types::{
    OverrideDecision, OverrideSelector, RequestBindings, Resolution, ResolutionSource,
    RoutingRequest, StrategyOverride,
};

#[must_use]
pub fn request_override(raw: StrategyOverride<'_>) -> OverrideDecision {
    let text = match raw {
        StrategyOverride::Absent => return OverrideDecision::NoOverride,
        StrategyOverride::Text(text) => text,
        StrategyOverride::NotText => {
            return OverrideDecision::Ignored {
                raw: None,
                supported: supported_overrides(),
            };
        }
    };
    match RoutingStrategy::parse(text) {
        Ok(strategy) if strategy.is_overridable() => OverrideDecision::Honored(strategy),
        Ok(_) | Err(_) => OverrideDecision::Ignored {
            raw: Some(text.to_owned()),
            supported: supported_overrides(),
        },
    }
}

pub fn override_selector(
    dispatch: &mut StrategyDispatch,
    strategy: RoutingStrategy,
) -> OverrideSelector {
    if strategy == dispatch.strategy {
        return OverrideSelector {
            selector: dispatch.default_selector,
            is_default: true,
        };
    }
    if let Some(cached) = dispatch.override_selectors.get(&strategy) {
        return OverrideSelector {
            selector: *cached,
            is_default: false,
        };
    }
    let selector = dispatch
        .build_selector(strategy, SelectorArgs::defaults_for(strategy), false)
        .map(|built| built.id);
    dispatch.override_selectors.insert(strategy, selector);
    OverrideSelector {
        selector,
        is_default: false,
    }
}

#[must_use]
pub fn override_pre_call_target(
    dispatch: &StrategyDispatch,
    strategy: RoutingStrategy,
    selector: Option<SelectorId>,
) -> Option<SelectorId> {
    let target = selector?;
    let cached = dispatch.override_selectors.get(&strategy)?;
    if *cached == Some(target) {
        return Some(target);
    }
    None
}

#[must_use]
pub fn globally_registered_strategies(
    dispatch: &StrategyDispatch,
    groups: &RoutingGroups,
) -> BTreeSet<RoutingStrategy> {
    let mut strategies = groups.globally_registered();
    strategies.insert(dispatch.strategy);
    strategies
}

#[must_use]
pub fn bind_override_selector(
    dispatch: &StrategyDispatch,
    strategy: RoutingStrategy,
    selector: Option<SelectorId>,
    groups: &RoutingGroups,
    bindings: Option<&mut RequestBindings>,
) -> bool {
    let (Some(target), Some(request_bindings)) = (selector, bindings) else {
        return false;
    };
    if globally_registered_strategies(dispatch, groups).contains(&strategy) {
        return false;
    }
    request_bindings.dynamic_callbacks.push(target);
    true
}

pub fn resolve(
    dispatch: &mut StrategyDispatch,
    request: &mut RoutingRequest<'_>,
    groups: &RoutingGroups,
) -> Resolution {
    if let OverrideDecision::Honored(strategy) = request_override(request.strategy) {
        let overridden = override_selector(dispatch, strategy);
        let bound = bind_override_selector(
            dispatch,
            strategy,
            overridden.selector,
            groups,
            request.bindings.as_deref_mut(),
        );
        return Resolution {
            strategy,
            selector: overridden.selector,
            source: ResolutionSource::RequestOverride,
            bound,
        };
    }
    if let Some(route) = groups.route(request.resolved_model) {
        let (strategy, selector) = match route.strategy {
            GroupStrategy::Priority => (RoutingStrategy::SimpleShuffle, None),
            GroupStrategy::Routing(strategy) => (strategy, route.selector),
        };
        return Resolution {
            strategy,
            selector,
            source: ResolutionSource::RoutingGroup(route.name.clone()),
            bound: false,
        };
    }
    Resolution {
        strategy: dispatch.strategy,
        selector: dispatch.default_selector,
        source: ResolutionSource::Default,
        bound: false,
    }
}

fn supported_overrides() -> Vec<RoutingStrategy> {
    let mut supported: Vec<RoutingStrategy> = VALID_ROUTING_STRATEGIES
        .iter()
        .copied()
        .filter(|strategy| strategy.is_overridable())
        .collect();
    supported.sort_by_key(|strategy| strategy.wire_name());
    supported
}
