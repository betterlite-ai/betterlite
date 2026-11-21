pub mod customer;
pub mod customer_gate;
pub mod entity;
pub mod gauge;
pub mod inputs;
pub mod key;
pub mod lists;
pub mod org;
pub mod remaining_budget;
pub mod reset_hours;
pub mod small_gauges;
pub mod team;
pub mod user;

#[path = "budget_gauges/tests/key_label_reuse.rs"]
mod key_label_reuse;
#[path = "budget_gauges/tests/support.rs"]
mod test_support;

use std::error::Error as StdError;

use crate::prometheus_helpers::bounded_series_tracker::TrackerError;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;

// the hours come from a JSON-free float, so a non-finite value is a bug this error reports instead of a gauge sample
#[derive(Debug, thiserror::Error)]
pub enum BudgetGaugeError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error(transparent)]
    Series(#[from] TrackerError<MetricError>),
    #[error("{hours} hours until the budget reset do not fit a gauge")]
    HoursNotFinite { hours: f64 },
}
