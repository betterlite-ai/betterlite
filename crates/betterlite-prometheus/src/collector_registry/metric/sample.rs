use std::time::SystemTime;

use rust_decimal::prelude::ToPrimitive as _;

use crate::collector_factory::collector::MetricValue;

use super::{Bucket, MISSING_LABEL};

// one label set's samples: the running total, the observation count, the cumulative bucket counts and when the series was created
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sample {
    pub total: MetricValue,
    pub count: u64,
    pub buckets: Vec<u64>,
    pub created: SystemTime,
}

impl Sample {
    // prometheus_client counts an observation in every bucket whose upper bound it does not exceed
    pub(super) fn observe(&mut self, value: MetricValue, bounds: &[Bucket]) {
        self.total = added(self.total, value);
        self.count = self.count.saturating_add(1);
        for (count, bound) in self.buckets.iter_mut().zip(bounds) {
            if within(value, *bound) {
                *count = count.saturating_add(1);
            }
        }
    }
}

// the IEEE rules MetricValue spells out: NaN wins, opposing infinities make NaN, one infinity survives
#[must_use]
pub fn added(left: MetricValue, right: MetricValue) -> MetricValue {
    match (left, right) {
        (MetricValue::NotANumber, _)
        | (_, MetricValue::NotANumber)
        | (MetricValue::PositiveInfinity, MetricValue::NegativeInfinity)
        | (MetricValue::NegativeInfinity, MetricValue::PositiveInfinity) => MetricValue::NotANumber,
        (MetricValue::PositiveInfinity, _) | (_, MetricValue::PositiveInfinity) => {
            MetricValue::PositiveInfinity
        }
        (MetricValue::NegativeInfinity, _) | (_, MetricValue::NegativeInfinity) => {
            MetricValue::NegativeInfinity
        }
        // a total past Decimal's range becomes an infinity, not a silently capped number
        (MetricValue::Finite(start), MetricValue::Finite(delta)) => {
            match start.checked_add(delta) {
                Some(sum) => MetricValue::Finite(sum),
                None if start.is_sign_negative() => MetricValue::NegativeInfinity,
                None => MetricValue::PositiveInfinity,
            }
        }
    }
}

// a MetricValue as the double the text renderer writes and the multiprocess files hold
#[must_use]
pub fn value_of(value: MetricValue) -> f64 {
    match value {
        MetricValue::Finite(number) => number.to_f64().unwrap_or(f64::NAN),
        MetricValue::PositiveInfinity => f64::INFINITY,
        MetricValue::NegativeInfinity => f64::NEG_INFINITY,
        MetricValue::NotANumber => f64::NAN,
    }
}

// prometheus_client counts an observation in every bucket whose upper bound it does not exceed
#[must_use]
pub fn within(value: MetricValue, bound: Bucket) -> bool {
    match value {
        MetricValue::Finite(number) => number
            .to_f64()
            .is_some_and(|magnitude| magnitude <= bound.key()),
        MetricValue::PositiveInfinity => matches!(bound, Bucket::Infinite),
        MetricValue::NegativeInfinity => true,
        MetricValue::NotANumber => false,
    }
}

pub(crate) fn label_key(label_values: &[Option<String>]) -> Vec<String> {
    label_values
        .iter()
        .map(|value| value.clone().unwrap_or_else(|| MISSING_LABEL.to_owned()))
        .collect()
}
