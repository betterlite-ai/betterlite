#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::{MetricUpdate, MetricValue};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use rust_decimal::Decimal;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::prometheus_exporter::{PrometheusCollectors, PrometheusExporterSettings};
use crate::prometheus_label_schema::PrometheusLabelSettings;

use super::counters::{
    increment_callback_logging_failure, increment_deployment_cooled_down,
    record_managed_batch_created,
};
use super::requests::{BatchCreatedLabels, CooledDownLabels};
use super::support::{
    ProbeGauges, capture_logs, expected_labels, inc_amount, labels_of, metric_of, update_of,
};

#[test]
fn cooled_down_increments_with_five_sanitized_positional_values() {
    let gauges = ProbeGauges::new();
    increment_deployment_cooled_down(
        &gauges,
        CooledDownLabels {
            litellm_model_name: "gpt-4o",
            model_id: "model-1",
            api_base: "https://api.example",
            api_provider: "openai",
            exception_status: "line\nbreak\"quoted\"",
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_cooled_down");
    assert_eq!(update_of(&events[0]), inc_amount());
    assert_eq!(
        labels_of(&events[0]),
        &expected_labels(
            "litellm_deployment_cooled_down",
            &[
                ("litellm_model_name", "gpt-4o"),
                ("model_id", "model-1"),
                ("api_base", "https://api.example"),
                ("api_provider", "openai"),
                ("exception_status", "line break\"quoted\""),
            ],
        )
    );
}

#[test]
fn cooled_down_escapes_backslashes() {
    let gauges = ProbeGauges::new();
    increment_deployment_cooled_down(
        &gauges,
        CooledDownLabels {
            litellm_model_name: "m",
            model_id: "i",
            api_base: "b",
            api_provider: "p",
            exception_status: "a\\b",
        },
    );
    let events = gauges.updates();
    assert_eq!(
        labels_of(&events[0])
            .get("exception_status")
            .map(String::as_str),
        Some("a\\\\b")
    );
}

#[test]
fn cooled_down_keeps_only_the_names_the_metric_still_declares() {
    let gauges = ProbeGauges::new().replacing_labels(
        "litellm_deployment_cooled_down",
        &["litellm_model_name", "exception_status"],
    );
    increment_deployment_cooled_down(
        &gauges,
        CooledDownLabels {
            litellm_model_name: "gpt-4o",
            model_id: "model-1",
            api_base: "https://api.example",
            api_provider: "openai",
            exception_status: "400",
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[0]);
    assert_eq!(labels.len(), 2);
    assert_eq!(
        labels.get("litellm_model_name").map(String::as_str),
        Some("gpt-4o")
    );
    assert_eq!(
        labels.get("exception_status").map(String::as_str),
        Some("400")
    );
}

fn registered() -> (CollectorRegistry, PrometheusCollectors) {
    let registry = CollectorRegistry::default();
    let collectors = PrometheusCollectors::register(
        &PrometheusExporterSettings::default(),
        &PrometheusLabelSettings::with_defaults(),
        &registry,
    )
    .expect("the collectors register");
    (registry, collectors)
}

fn callback_total(registry: &CollectorRegistry, callback_name: &str) -> Option<MetricValue> {
    registry
        .get("litellm_callback_logging_failures_metric")?
        .series(&[Some(callback_name.to_owned())])
        .map(|sample| sample.total)
}

#[test]
fn callback_failure_increments_the_callback_metric_only() {
    let (registry, collectors) = registered();
    increment_callback_logging_failure(
        &collectors.litellm_callback_logging_failures_metric,
        "langfuse",
    );
    assert_eq!(
        callback_total(&registry, "langfuse"),
        Some(MetricValue::Finite(Decimal::ONE))
    );
}

#[test]
fn the_exporter_implements_the_callback_failure_counter_trait() {
    let (registry, collectors) = registered();
    collectors.increment_callback_logging_failure("s3");
    assert_eq!(
        callback_total(&registry, "s3"),
        Some(MetricValue::Finite(Decimal::ONE))
    );
}

#[test]
fn managed_batch_created_increments_the_five_fixed_labels() {
    let gauges = ProbeGauges::new();
    record_managed_batch_created(
        &gauges,
        BatchCreatedLabels {
            model: Some("gpt-4o"),
            api_provider: Some("openai"),
            user: Some("user-1"),
            user_email: Some("user@example.com"),
            api_key_alias: None,
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_managed_batch_created_total");
    assert_eq!(update_of(&events[0]), inc_amount());
    assert_eq!(
        labels_of(&events[0]),
        &expected_labels(
            "litellm_managed_batch_created_total",
            &[
                ("model", "gpt-4o"),
                ("api_provider", "openai"),
                ("user", "user-1"),
                ("user_email", "user@example.com"),
                ("api_key_alias", "None"),
            ],
        )
    );
}

#[test]
fn managed_batch_created_logs_a_collector_error_and_records_nothing() {
    let gauges =
        ProbeGauges::new().replacing_labels("litellm_managed_batch_created_total", &["model"]);
    let (captured, _guard) = capture_logs(tracing::Level::WARN);
    record_managed_batch_created(
        &gauges,
        BatchCreatedLabels {
            model: Some("gpt-4o"),
            api_provider: Some("openai"),
            user: Some("user-1"),
            user_email: Some("user@example.com"),
            api_key_alias: Some("alias"),
        },
    );
    assert!(gauges.updates().is_empty());
    assert!(
        captured
            .text()
            .contains("Error recording batch created metric:")
    );
}

#[test]
fn callback_failure_uses_only_the_callback_label() {
    let (registry, collectors) = registered();
    CallbackFailureCounter::increment_callback_logging_failure(&collectors, "custom");
    assert_eq!(
        callback_total(&registry, "custom"),
        Some(MetricValue::Finite(Decimal::ONE))
    );
}

#[test]
fn cooled_down_amount_is_one() {
    let gauges = ProbeGauges::new();
    increment_deployment_cooled_down(
        &gauges,
        CooledDownLabels {
            litellm_model_name: "m",
            model_id: "i",
            api_base: "b",
            api_provider: "p",
            exception_status: "e",
        },
    );
    assert_eq!(
        update_of(&gauges.updates()[0]),
        MetricUpdate::Inc(MetricValue::Finite(Decimal::ONE))
    );
}
