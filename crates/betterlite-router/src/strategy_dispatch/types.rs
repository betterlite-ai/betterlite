use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::selector::{Selector, SelectorId};
use crate::strategy_dispatch::strategy::RoutingStrategy;

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
pub struct IgnoredOverride {
    pub raw: Option<String>,
    pub supported: Vec<RoutingStrategy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub strategy: RoutingStrategy,
    pub selector: Option<SelectorId>,
    pub source: ResolutionSource,
    pub bound: bool,
    pub ignored_override: Option<IgnoredOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionSource {
    RequestOverride,
    RoutingGroup(String),
    Default,
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
