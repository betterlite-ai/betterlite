use crate::strategy_dispatch::selector::{Selector, SelectorId};
use crate::strategy_dispatch::strategy::RoutingStrategy;

#[derive(Debug, Default)]
pub(crate) struct SelectorRegistry {
    pub(crate) callbacks: Vec<SelectorId>,
    pub(crate) input_callbacks: Vec<SelectorId>,
}

impl SelectorRegistry {
    pub(crate) fn register(&mut self, selector: &Selector) {
        if !self.callbacks.contains(&selector.id) {
            self.callbacks.push(selector.id);
        }
        // LiteLLM subscribes only least-busy to the input events; the others count on success/failure
        if selector.strategy == RoutingStrategy::LeastBusy
            && !self.input_callbacks.contains(&selector.id)
        {
            self.input_callbacks.push(selector.id);
        }
    }

    pub(crate) fn unregister(&mut self, id: SelectorId) {
        self.callbacks.retain(|registered| *registered != id);
        self.input_callbacks.retain(|registered| *registered != id);
    }
}
