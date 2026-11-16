use std::sync::Arc;

use rust_decimal::Decimal;

use super::prometheus::DailySpendSource;
use super::store::TotalsStore;

#[derive(Clone)]
pub struct TotalsState {
    pub store: Option<Arc<dyn TotalsStore>>,
    pub prometheus: Option<Arc<dyn DailySpendSource>>,
    pub max_budget: Decimal,
}
