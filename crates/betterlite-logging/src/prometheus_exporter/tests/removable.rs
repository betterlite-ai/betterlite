use rust_decimal::Decimal;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};

use crate::prometheus_exporter::PrometheusExporterSettings;
use crate::prometheus_helpers::bounded_series_tracker::{ChildRemoval, RemovableMetric};

use super::{default_label_settings, registered};

const PROVIDER_BUDGET: &str = "litellm_provider_remaining_budget_metric";

#[test]
fn a_real_metric_removes_its_series_once_and_reports_it_absent_after() {
    let (registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    let series = [Some("openai".to_owned())];
    collectors
        .litellm_provider_remaining_budget_metric
        .labels(&[("api_provider", "openai")])
        .set(MetricValue::Finite(Decimal::new(1, 0)));
    let registered_metric = registry.get(PROVIDER_BUDGET).expect("registered");
    let _sample = registered_metric
        .series(&series)
        .expect("the series exists");
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&series)
            .expect("removal succeeds"),
        ChildRemoval::Removed
    );
    assert!(
        registry
            .get(PROVIDER_BUDGET)
            .expect("registered")
            .series(&series)
            .is_none()
    );
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&series)
            .expect("removal succeeds"),
        ChildRemoval::Absent
    );
}

#[test]
fn a_wrong_label_count_is_unsupported() {
    let (_registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&[])
            .expect("removal succeeds"),
        ChildRemoval::Unsupported
    );
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&[Some("openai".to_owned()), Some("anthropic".to_owned())])
            .expect("removal succeeds"),
        ChildRemoval::Unsupported
    );
}

#[test]
fn a_no_op_metric_is_unsupported() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_exclude_metrics = vec![PROVIDER_BUDGET.to_owned()];
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&[])
            .expect("removal succeeds"),
        ChildRemoval::Unsupported
    );
}

#[test]
fn a_label_filtered_metric_is_unsupported() {
    let mut settings = PrometheusExporterSettings::default();
    settings.prometheus_exclude_labels = vec!["api_provider".to_owned()];
    let (_registry, collectors) = registered(&settings, &default_label_settings());
    assert_eq!(
        collectors
            .litellm_provider_remaining_budget_metric
            .remove_child(&[Some("openai".to_owned())])
            .expect("removal succeeds"),
        ChildRemoval::Unsupported
    );
}
