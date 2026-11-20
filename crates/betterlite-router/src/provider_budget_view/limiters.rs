use std::sync::Arc;

use super::view::ProviderBudgetReads;

// RouterBudgetLimiting's exact class; virtual-key model budgets are separate callback subclasses
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackKind {
    RouterBudget,
    KeyBudget,
    Other,
}

pub struct OptionalCallback {
    pub kind: CallbackKind,
    pub reads: Arc<dyn ProviderBudgetReads>,
}

// _get_router_deployment_budget_limiter: the cached logger answers first, else the first exact router-budget callback
#[must_use]
pub fn select_router_budget_limiter(
    cached: Option<Arc<dyn ProviderBudgetReads>>,
    optional_callbacks: &[OptionalCallback],
) -> Option<Arc<dyn ProviderBudgetReads>> {
    cached.or_else(|| {
        optional_callbacks
            .iter()
            .find(|callback| callback.kind == CallbackKind::RouterBudget)
            .map(|callback| Arc::clone(&callback.reads))
    })
}
