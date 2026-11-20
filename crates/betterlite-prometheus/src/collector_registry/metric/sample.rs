use num_traits::CheckedAdd as _;
use num_traits::ToPrimitive as _;

use crate::collector_factory::collector::MetricValue;

use super::Bucket;

// one label set's samples: the running total, the observation count and the cumulative bucket counts
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sample {
    pub total: MetricValue,
    pub count: u64,
    pub buckets: Vec<u64>,
}

// the IEEE rules MetricValue spells out: NaN wins, opposing infinities make NaN, one infinity survives
pub fn added(left: MetricValue, right: MetricValue) -> MetricValue {
    match (left, right) {
        (MetricValue::NotANumber, _) | (_, MetricValue::NotANumber) => MetricValue::NotANumber,
        (MetricValue::PositiveInfinity, MetricValue::NegativeInfinity)
        | (MetricValue::NegativeInfinity, MetricValue::PositiveInfinity) => MetricValue::NotANumber,
        (MetricValue::PositiveInfinity, _) | (_, MetricValue::PositiveInfinity) => {
            MetricValue::PositiveInfinity
        }
        (MetricValue::NegativeInfinity, _) | (_, MetricValue::NegativeInfinity) => {
            MetricValue::NegativeInfinity
        }
        // a total past Decimal's range becomes an infinity, not a silently capped number
        (MetricValue::Finite(left), MetricValue::Finite(right)) => match left.checked_add(right) {
            Some(sum) => MetricValue::Finite(sum),
            None if left.is_sign_negative() => MetricValue::NegativeInfinity,
            None => MetricValue::PositiveInfinity,
        },
    }
}

// prometheus_client counts an observation in every bucket whose upper bound it does not exceed
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
