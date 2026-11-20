use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::usage_object::positive_value;

#[derive(Clone, Copy, Debug)]
pub enum SparseValue<'value> {
    Json(&'value Json),
    Integer(i64),
}

impl SparseValue<'_> {
    #[must_use]
    pub fn amount(self) -> Option<MetricValue> {
        match self {
            Self::Json(value) => positive_value(value),
            Self::Integer(tokens) => {
                (tokens > 0).then(|| MetricValue::Finite(Decimal::from(tokens)))
            }
        }
    }
}

pub fn inc_sparse_usage_counters<C: UsageCounters>(
    counters: &C,
    entries: &[(UsageMetric, Option<SparseValue<'_>>)],
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    for (metric, value) in entries {
        let Some(amount) = value.and_then(SparseValue::amount) else {
            continue;
        };
        counters.inc(*metric, labels, amount, now)?;
    }
    Ok(())
}
