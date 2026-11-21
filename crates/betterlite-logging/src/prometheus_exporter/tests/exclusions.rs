use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::collector_factory::config::PrometheusMetricsConfig;

use crate::prometheus_exporter::PrometheusExporterSettings;

use super::{default_label_settings, registered};

#[test]
fn an_excluded_metric_is_a_no_op_and_absent_from_the_registry() {
    let settings = PrometheusExporterSettings {
        prometheus_exclude_metrics: vec!["litellm_spend_metric".to_owned()],
        ..PrometheusExporterSettings::default()
    };
    let (registry, collectors) = registered(&settings, &default_label_settings());
    assert!(matches!(
        collectors.litellm_spend_metric,
        MetricLike::NoOp(_)
    ));
    assert!(registry.get("litellm_spend_metric").is_none());
    assert_eq!(registry.descriptions().len(), 80);
    assert!(collectors.exclude_metrics.contains("litellm_spend_metric"));
}

#[test]
fn an_excluded_label_is_dropped_from_schema_labels() {
    let settings = PrometheusExporterSettings {
        prometheus_exclude_labels: vec!["model".to_owned()],
        ..PrometheusExporterSettings::default()
    };
    let (registry, collectors) = registered(&settings, &default_label_settings());
    let metric = registry
        .get("litellm_spend_metric")
        .expect("the collector is registered");
    let labels = &metric.description.label_names;
    assert!(!labels.iter().any(|label| label == "model"));
    assert!(labels.iter().any(|label| label == "hashed_api_key"));
    assert!(matches!(
        collectors.litellm_spend_metric,
        MetricLike::Excluded(_)
    ));
    assert!(collectors.exclude_labels.contains("model"));
}

#[test]
fn an_excluded_label_is_dropped_from_fixed_labels() {
    let settings = PrometheusExporterSettings {
        prometheus_exclude_labels: vec!["status".to_owned()],
        ..PrometheusExporterSettings::default()
    };
    let (registry, _collectors) = registered(&settings, &default_label_settings());
    let metric = registry
        .get("litellm_guardrail_latency_seconds")
        .expect("the collector is registered");
    assert_eq!(
        metric.description.label_names,
        ["guardrail_name", "error_type", "hook_type"]
    );
}

#[test]
fn an_excluded_label_on_a_no_label_metric_leaves_it_a_real_collector() {
    let settings = PrometheusExporterSettings {
        prometheus_exclude_labels: vec!["model".to_owned()],
        ..PrometheusExporterSettings::default()
    };
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert!(matches!(
        collectors.litellm_total_users_metric,
        MetricLike::Real(_)
    ));
}

#[test]
fn include_labels_narrow_the_named_metric_only() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: vec![PrometheusMetricsConfig {
            group: "experiments".to_owned(),
            metrics: vec!["litellm_spend_metric".to_owned()],
            include_labels: vec!["hashed_api_key".to_owned()],
        }],
        ..PrometheusExporterSettings::default()
    };
    let (registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(registry.descriptions().len(), 1);
    let metric = registry
        .get("litellm_spend_metric")
        .expect("the collector is registered");
    assert_eq!(metric.description.label_names, ["hashed_api_key"]);
    assert_eq!(
        collectors.label_filters.get("litellm_spend_metric"),
        Some(&vec!["hashed_api_key".to_owned()])
    );
    assert!(matches!(
        collectors.litellm_requests_metric,
        MetricLike::NoOp(_)
    ));
}

#[test]
fn include_labels_keep_the_schema_order_of_the_kept_labels() {
    let settings = PrometheusExporterSettings {
        prometheus_metrics_config: vec![PrometheusMetricsConfig {
            group: "experiments".to_owned(),
            metrics: vec!["litellm_cache_hits_metric".to_owned()],
            include_labels: vec!["api_key_alias".to_owned(), "model".to_owned()],
        }],
        ..PrometheusExporterSettings::default()
    };
    let (registry, _collectors) = registered(&settings, &default_label_settings());
    let metric = registry
        .get("litellm_cache_hits_metric")
        .expect("the collector is registered");
    assert_eq!(metric.description.label_names, ["model", "api_key_alias"]);
}
