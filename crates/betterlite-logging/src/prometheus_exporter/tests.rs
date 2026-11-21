#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::CollectorDescription;

use crate::prometheus_exporter::{Handle, PrometheusCollectors, PrometheusExporterSettings};
use crate::prometheus_label_schema::PrometheusLabelSettings;

mod buckets;
mod cache;
mod duplicates;
mod exclusions;
mod fields;
mod helpful_text;
mod order;
mod removable;

fn description(handle: &Handle) -> &CollectorDescription {
    match handle {
        MetricLike::Real(metric) => &metric.description,
        MetricLike::Excluded(excluded) => &excluded.metric.description,
        MetricLike::NoOp(_) => panic!("the collector is a no-op"),
    }
}

fn default_label_settings() -> PrometheusLabelSettings {
    PrometheusLabelSettings::with_defaults()
}

fn registered(
    settings: &PrometheusExporterSettings,
    labels: &PrometheusLabelSettings,
) -> (CollectorRegistry, PrometheusCollectors) {
    let registry = CollectorRegistry::default();
    let collectors = PrometheusCollectors::register(settings, labels, &registry)
        .expect("the collectors register");
    (registry, collectors)
}
