use std::collections::BTreeMap;

use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::registry::SelectorRegistry;
use crate::strategy_dispatch::selector::{Selector, SelectorId, SelectorState};
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::ArgsUpdate;

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

    pub(crate) fn build_selector(
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
