use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemainingBudget {
    Unlimited,
    Amount(Decimal),
}

impl RemainingBudget {
    #[must_use]
    pub const fn metric_value(self) -> MetricValue {
        match self {
            Self::Unlimited => MetricValue::PositiveInfinity,
            Self::Amount(amount) => MetricValue::Finite(amount),
        }
    }
}

// LiteLLM's _safe_get_remaining_budget: no limit is infinity, an unrecorded spend leaves the whole limit, and overspend goes negative
#[must_use]
pub fn remaining_budget(max_budget: Option<Decimal>, spend: Option<Decimal>) -> RemainingBudget {
    match (max_budget, spend) {
        (None, _) => RemainingBudget::Unlimited,
        (Some(limit), None) => RemainingBudget::Amount(limit),
        (Some(limit), Some(used)) => RemainingBudget::Amount(limit.saturating_sub(used)),
    }
}

mod tests;
