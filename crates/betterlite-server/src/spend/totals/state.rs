use std::sync::Arc;

use rust_decimal::Decimal;

use super::store::TotalsStore;

pub struct TotalsState<S> {
    pub store: Option<Arc<dyn TotalsStore>>,
    pub prometheus: Option<Arc<S>>,
    pub max_budget: Decimal,
}

impl<S> Clone for TotalsState<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.as_ref().map(Arc::clone),
            prometheus: self.prometheus.as_ref().map(Arc::clone),
            max_budget: self.max_budget,
        }
    }
}
