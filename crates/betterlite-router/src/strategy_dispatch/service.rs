use std::collections::{BTreeMap, BTreeSet};

use rand::Rng;

use crate::simple_shuffle::{Candidate, pick};
use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroups};
use crate::strategy_dispatch::registry::SelectorRegistry;
use crate::strategy_dispatch::selector::{Selector, SelectorId, SelectorState};
use crate::strategy_dispatch::strategy::{RoutingStrategy, VALID_ROUTING_STRATEGIES};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestBindings {
    pub dynamic_callbacks: Vec<SelectorId>,
}

pub struct RoutingRequest<'request> {
    pub resolved_model: &'request str,
    pub strategy: StrategyOverride<'request>,
    pub bindings: Option<&'request mut RequestBindings>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyOverride<'request> {
    Absent,
    Text(&'request str),
    NotText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideDecision {
    NoOverride,
    Honored(RoutingStrategy),
    Ignored {
        raw: Option<String>,
        supported: Vec<RoutingStrategy>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideSelector {
    pub selector: Option<SelectorId>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub strategy: RoutingStrategy,
    pub selector: Option<SelectorId>,
    pub source: ResolutionSource,
    pub bound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionSource {
    RequestOverride,
    RoutingGroup(String),
    Default,
}

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

#[derive(Debug)]
pub enum ArgsUpdate {
    Rebuilt {
        previous: SelectorId,
        selector: Selector,
    },
    KeptPrevious {
        error: StrategyDispatchError,
    },
    NoSelector,
    Lar1Delegated,
}

#[derive(Debug)]
pub struct StrategyDispatch {
    pub(crate) strategy: RoutingStrategy,
    pub(crate) args: SelectorArgs,
    pub(crate) default_selector: Option<SelectorId>,
    pub(crate) selectors: Vec<Selector>,
    pub(crate) override_selectors: BTreeMap<RoutingStrategy, Option<SelectorId>>,
    pub(crate) registry: SelectorRegistry,
}

impl StrategyDispatch {
    pub fn new(
        strategy: RoutingStrategy,
        args: Option<SelectorArgs>,
    ) -> Result<Self, StrategyDispatchError> {
        let mut dispatch = Self {
            strategy,
            args: SelectorArgs::Fixed,
            default_selector: None,
            selectors: Vec::new(),
            override_selectors: BTreeMap::new(),
            registry: SelectorRegistry::default(),
        };
        dispatch.init(strategy, args)?;
        Ok(dispatch)
    }

    pub fn init(
        &mut self,
        strategy: RoutingStrategy,
        args: Option<SelectorArgs>,
    ) -> Result<SelectorState, StrategyDispatchError> {
        let resolved = SelectorArgs::for_strategy(strategy, args)?;
        let mut retired: Vec<SelectorId> = self
            .override_selectors
            .values()
            .flatten()
            .copied()
            .collect();
        retired.extend(self.default_selector);
        self.override_selectors.clear();
        self.default_selector = None;
        for selector in retired {
            self.retire(selector);
        }
        self.strategy = strategy;
        self.args = resolved;
        if let Some(selector) = self.build_selector(strategy, resolved, true) {
            self.default_selector = Some(selector.id);
        }
        Ok(self.state())
    }

    #[must_use]
    pub fn apply_updated_args(&mut self, args: Option<SelectorArgs>) -> ArgsUpdate {
        if self.strategy == RoutingStrategy::Lar1 {
            return ArgsUpdate::Lar1Delegated;
        }
        let Some(previous) = self.default_selector else {
            return ArgsUpdate::NoSelector;
        };
        let resolved = match SelectorArgs::for_strategy(self.strategy, args) {
            Ok(resolved) => resolved,
            Err(error) => return ArgsUpdate::KeptPrevious { error },
        };
        // the rebuilt selector is registered before the previous one retires, so it keeps receiving events
        let Some(selector) = self.build_selector(self.strategy, resolved, true) else {
            return ArgsUpdate::NoSelector;
        };
        self.args = resolved;
        self.default_selector = Some(selector.id);
        self.retire(previous);
        ArgsUpdate::Rebuilt { previous, selector }
    }

    pub fn build_group_selector(
        &mut self,
        strategy: RoutingStrategy,
        args: Option<SelectorArgs>,
    ) -> Result<Option<SelectorId>, StrategyDispatchError> {
        let resolved = SelectorArgs::for_strategy(strategy, args)?;
        Ok(self
            .build_selector(strategy, resolved, false)
            .map(|selector| selector.id))
    }

    pub fn register_selector(&mut self, id: SelectorId) -> Result<(), StrategyDispatchError> {
        let Some(selector) = self.selectors.get(id.0).copied() else {
            return Err(StrategyDispatchError::UnknownSelector { id });
        };
        self.registry.register(&selector);
        Ok(())
    }

    pub fn unregister(
        &mut self,
        selectors: &[SelectorId],
    ) -> Result<SelectorState, StrategyDispatchError> {
        for selector in selectors {
            if selector.0 >= self.selectors.len() {
                return Err(StrategyDispatchError::UnknownSelector { id: *selector });
            }
        }
        for selector in selectors {
            self.retire(*selector);
        }
        Ok(self.state())
    }

    #[must_use]
    pub fn request_override(&self, raw: StrategyOverride<'_>) -> OverrideDecision {
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

    pub fn override_selector(&mut self, strategy: RoutingStrategy) -> OverrideSelector {
        if strategy == self.strategy {
            return OverrideSelector {
                selector: self.default_selector,
                is_default: true,
            };
        }
        if let Some(cached) = self.override_selectors.get(&strategy) {
            return OverrideSelector {
                selector: *cached,
                is_default: false,
            };
        }
        let selector = self
            .build_selector(strategy, SelectorArgs::defaults_for(strategy), false)
            .map(|built| built.id);
        self.override_selectors.insert(strategy, selector);
        OverrideSelector {
            selector,
            is_default: false,
        }
    }

    #[must_use]
    pub fn override_pre_call_target(
        &self,
        strategy: RoutingStrategy,
        selector: Option<SelectorId>,
    ) -> Option<SelectorId> {
        let target = selector?;
        let cached = self.override_selectors.get(&strategy)?;
        if *cached == Some(target) {
            return Some(target);
        }
        None
    }

    #[must_use]
    pub fn globally_registered_strategies(
        &self,
        groups: &RoutingGroups,
    ) -> BTreeSet<RoutingStrategy> {
        let mut strategies = groups.globally_registered();
        strategies.insert(self.strategy);
        strategies
    }

    #[must_use]
    pub fn bind_override_selector(
        &self,
        strategy: RoutingStrategy,
        selector: Option<SelectorId>,
        groups: &RoutingGroups,
        bindings: Option<&mut RequestBindings>,
    ) -> bool {
        let (Some(target), Some(request_bindings)) = (selector, bindings) else {
            return false;
        };
        if self
            .globally_registered_strategies(groups)
            .contains(&strategy)
        {
            return false;
        }
        request_bindings.dynamic_callbacks.push(target);
        true
    }

    pub fn resolve(
        &mut self,
        request: &mut RoutingRequest<'_>,
        groups: &RoutingGroups,
    ) -> Resolution {
        if let OverrideDecision::Honored(strategy) = self.request_override(request.strategy) {
            let overridden = self.override_selector(strategy);
            let bound = self.bind_override_selector(
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
            strategy: self.strategy,
            selector: self.default_selector,
            source: ResolutionSource::Default,
            bound: false,
        }
    }

    pub fn select<R>(
        &mut self,
        request: &mut RoutingRequest<'_>,
        groups: &RoutingGroups,
        candidates: &[Candidate],
        random: &mut R,
    ) -> Result<SelectionOutcome, StrategyDispatchError>
    where
        R: Rng,
    {
        let resolution = self.resolve(request, groups);
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
        &mut self,
        request: &mut RoutingRequest<'_>,
        groups: &RoutingGroups,
        deployments: &'deployments [Deployment],
    ) -> Option<ReadPrefetchPlan<'deployments, Deployment>> {
        let resolution = self.resolve(request, groups);
        if deployments.is_empty() {
            return None;
        }
        let usage_selector = if resolution.strategy == RoutingStrategy::UsageBasedRoutingV2
            && let Some(selector) = resolution.selector.and_then(|id| self.selectors.get(id.0))
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
    pub fn state(&self) -> SelectorState {
        SelectorState {
            selector: self
                .default_selector
                .and_then(|id| self.selectors.get(id.0).copied()),
            callbacks: self.registered(&self.registry.callbacks),
            input_callbacks: self.registered(&self.registry.input_callbacks),
        }
    }

    fn registered(&self, ids: &[SelectorId]) -> Vec<Selector> {
        let mut selectors = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(selector) = self.selectors.get(id.0) {
                selectors.push(*selector);
            }
        }
        selectors
    }

    fn build_selector(
        &mut self,
        strategy: RoutingStrategy,
        args: SelectorArgs,
        register: bool,
    ) -> Option<Selector> {
        if !strategy.has_selector() {
            return None;
        }
        let selector = Selector {
            id: SelectorId(self.selectors.len()),
            strategy,
            args,
            retired: false,
        };
        self.selectors.push(selector);
        if register {
            self.registry.register(&selector);
        }
        Some(selector)
    }

    fn retire(&mut self, id: SelectorId) {
        if let Some(selector) = self.selectors.get_mut(id.0) {
            selector.retired = true;
        }
        self.registry.unregister(id);
    }
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

fn supported_overrides() -> Vec<RoutingStrategy> {
    let mut supported: Vec<RoutingStrategy> = VALID_ROUTING_STRATEGIES
        .iter()
        .copied()
        .filter(|strategy| strategy.is_overridable())
        .collect();
    supported.sort_by_key(|strategy| strategy.wire_name());
    supported
}
