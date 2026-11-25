use std::io;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prometheus::collector_factory::collector::{
    Collector as _, MetricUpdate, MetricValue,
};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::{
    Bucket, PrometheusMetric, prepare_buckets,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::{Action, Kind, MetricSpec, UpdateSpec};

pub(super) fn build(
    registry: &CollectorRegistry,
    specs: &[MetricSpec],
) -> Result<Vec<PrometheusMetric>, Box<HelperError>> {
    let mut metrics = Vec::with_capacity(specs.len());
    for spec in specs {
        metrics.push(build_one(registry, spec)?);
    }
    Ok(metrics)
}

fn build_one(
    registry: &CollectorRegistry,
    spec: &MetricSpec,
) -> Result<PrometheusMetric, Box<HelperError>> {
    let mut bounds = Vec::with_capacity(spec.buckets.len());
    for seconds in &spec.buckets {
        bounds.push(Bucket::Seconds(*seconds));
    }
    let label_names: Vec<&str> = spec.labels.iter().map(String::as_str).collect();
    // Counter._metric_init and Gauge._metric_init take no buckets; only Histogram validates them and adds the infinite bound
    let prepared = match spec.kind {
        Kind::Histogram => prepare_buckets(&bounds).or_value_error()?,
        Kind::Counter | Kind::Gauge => Vec::new(),
    };
    let metric = PrometheusMetric::new(
        spec.kind.collector_kind(),
        &spec.name,
        &spec.help,
        &label_names,
        prepared,
    )
    .with_multiprocess_mode(spec.mode.gauge_mode());
    registry.register(metric.clone()).or_value_error()?;
    Ok(metric)
}

pub(super) fn apply_updates(
    metrics: &[PrometheusMetric],
    updates: &[UpdateSpec],
) -> Result<(), Box<HelperError>> {
    for update in updates {
        let metric = metrics
            .get(update.metric)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "the case defines no metric at index {}",
                    update.metric
                ))
            })
            .or_value_error()?;
        let pairs = label_pairs(&metric.description.label_names, &update.labels);
        let handle = metric.labels(&pairs);
        let change = match update.action {
            Action::Inc => MetricUpdate::Inc(metric_value(update.value)),
            Action::Set => MetricUpdate::Set(metric_value(update.value)),
            Action::Observe => MetricUpdate::Observe(metric_value(update.value)),
            Action::Remove => MetricUpdate::Remove,
        };
        handle.update(change);
    }
    Ok(())
}

fn label_pairs<'names, 'values>(
    label_names: &'names [String],
    values: &'values [String],
) -> Vec<(&'names str, &'values str)> {
    let mut pairs = Vec::with_capacity(label_names.len());
    for (name, value) in label_names.iter().zip(values) {
        pairs.push((name.as_str(), value.as_str()));
    }
    pairs
}

fn metric_value(value: f64) -> MetricValue {
    match Decimal::from_f64(value) {
        Some(finite) => MetricValue::Finite(finite),
        None if value.is_nan() => MetricValue::NotANumber,
        None if value.is_sign_negative() => MetricValue::NegativeInfinity,
        None => MetricValue::PositiveInfinity,
    }
}
