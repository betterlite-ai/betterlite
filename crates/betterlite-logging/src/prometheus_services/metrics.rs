use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

pub(super) fn label_pairs<'names, 'text>(
    names: &'names [String],
    values: &[&'text str],
) -> Vec<(&'names str, &'text str)> {
    names
        .iter()
        .map(String::as_str)
        .zip(values.iter().copied())
        .collect()
}

pub(super) fn metric_value(amount: f64) -> MetricValue {
    if amount.is_nan() {
        return MetricValue::NotANumber;
    }
    if amount.is_infinite() {
        return if amount.is_sign_positive() {
            MetricValue::PositiveInfinity
        } else {
            MetricValue::NegativeInfinity
        };
    }
    match Decimal::from_f64(amount) {
        Some(value) => MetricValue::Finite(value),
        None if amount.is_sign_negative() => MetricValue::NegativeInfinity,
        None => MetricValue::PositiveInfinity,
    }
}
