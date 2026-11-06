#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_prometheus::collector_factory::{
    CollectorFactory, CollectorFactoryError, NON_ENUM_METRIC_LABELS, PrometheusMetricsConfig,
    all_defined_labels, is_metric_enabled, pretty_print_prometheus_config,
    pretty_print_validation_errors,
};
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use betterlite_prometheus::validation::{
    LabelValidationError, MetricValidationError, ValidationResults, ValidationSummary as _,
};

use crate::support::FakeSchema;

fn config(metrics: &[&str], labels: &[&str]) -> PrometheusMetricsConfig {
    PrometheusMetricsConfig {
        group: String::from("test"),
        metrics: metrics.iter().map(|metric| (*metric).to_owned()).collect(),
        include_labels: labels.iter().map(|label| (*label).to_owned()).collect(),
    }
}

fn schema() -> FakeSchema {
    FakeSchema::new(&[
        ("litellm_spend_metric", &["model", "team", "end_user"]),
        ("litellm_requests_metric", &["model", "team"]),
    ])
}

#[test]
fn prometheus_config_parses_metrics_and_label_filters() {
    let mut factory = CollectorFactory::new(schema());
    let configs = [
        config(&["litellm_spend_metric"], &["model", "team"]),
        config(&["litellm_requests_metric"], &[]),
    ];
    factory.parse_prometheus_config(&configs).unwrap();
    let enabled = &factory.enabled_metrics;
    assert_eq!(enabled.len(), 2);
    assert!(enabled.contains("litellm_spend_metric"));
    assert!(enabled.contains("litellm_requests_metric"));
    assert_eq!(
        factory.label_filters.get("litellm_spend_metric"),
        Some(&vec![String::from("model"), String::from("team")])
    );
    assert!(
        !factory
            .label_filters
            .contains_key("litellm_requests_metric")
    );
}

#[test]
fn prometheus_config_without_groups_leaves_every_metric_enabled() {
    let mut factory = CollectorFactory::new(schema());
    factory.parse_prometheus_config(&[]).unwrap();
    assert!(factory.enabled_metrics.is_empty());
    assert!(factory.label_filters.is_empty());
    assert!(is_metric_enabled(
        &factory.enabled_metrics,
        &factory.exclude_metrics,
        "litellm_spend_metric"
    ));
}

#[test]
fn prometheus_config_rejects_unknown_metric_names() {
    let mut factory = CollectorFactory::new(schema());
    let error = factory
        .parse_prometheus_config(&[config(&["not_a_metric"], &["model"])])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Invalid metric name: not_a_metric")
    );
    assert!(
        error
            .to_string()
            .contains("Configuration validation failed:")
    );
    let CollectorFactoryError::Configuration { results } = &error else {
        panic!("expected a typed configuration error");
    };
    assert!(results.has_errors());
    assert!(factory.label_filters.is_empty());
}

#[test]
fn prometheus_config_rejects_invalid_include_labels() {
    let mut factory = CollectorFactory::new(schema());
    let error = factory
        .parse_prometheus_config(&[config(&["litellm_spend_metric"], &["bogus"])])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Invalid labels for metric 'litellm_spend_metric': [\"bogus\"]")
    );
    assert!(factory.label_filters.is_empty());
}

#[test]
fn prometheus_config_skips_label_validation_for_an_invalid_metric() {
    let mut factory = CollectorFactory::new(schema());
    let error = factory
        .parse_prometheus_config(&[config(&["not_a_metric"], &["bogus"])])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Invalid metric name: not_a_metric")
    );
}

#[test]
fn prometheus_config_keeps_the_last_group_for_a_metric() {
    let mut factory = CollectorFactory::new(schema());
    let configs = [
        config(&["litellm_spend_metric"], &["model"]),
        config(&["litellm_spend_metric"], &["team"]),
    ];
    factory.parse_prometheus_config(&configs).unwrap();
    assert_eq!(
        factory.label_filters.get("litellm_spend_metric"),
        Some(&vec![String::from("team")])
    );
}

#[test]
fn prometheus_config_rejects_invalid_exclusions() {
    let mut factory = CollectorFactory::new(schema());
    let error = factory
        .parse_exclude_config(
            &[String::from("not_a_metric")],
            &[String::from("bogus")],
            &[],
            &[],
        )
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("Prometheus exclude configuration validation failed:"));
    assert!(message.contains("Invalid metric name in prometheus_exclude_metrics: not_a_metric"));
    assert!(message.contains("Invalid label name in prometheus_exclude_labels: bogus"));
    assert!(factory.exclude_metrics.is_empty());
    assert!(factory.exclude_labels.is_empty());
}

#[test]
fn prometheus_config_accepts_defined_exclusions_and_sanitized_custom_labels() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(
            &[String::from("litellm_spend_metric")],
            &[
                String::from("team"),
                String::from("Request_ID"),
                String::from("tag_env"),
            ],
            &[String::from("Request-ID")],
            &[String::from("env")],
        )
        .unwrap();
    assert!(factory.exclude_metrics.contains("litellm_spend_metric"));
    assert!(factory.exclude_labels.contains("team"));
    assert!(factory.exclude_labels.contains("Request_ID"));
    assert!(factory.exclude_labels.contains("tag_env"));
}

#[test]
fn prometheus_config_all_defined_labels_union_and_sanitize() {
    let labels = all_defined_labels(&[String::from("Request-ID")], &[String::from("my-tag")]);
    assert!(
        UserApiKeyLabelName::ALL
            .iter()
            .all(|name| labels.contains(name.as_str()))
    );
    assert!(
        NON_ENUM_METRIC_LABELS
            .iter()
            .all(|name| labels.contains(*name))
    );
    assert!(labels.contains("Request_ID"));
    assert!(labels.contains("tag_my_tag"));
    assert!(!labels.contains("Request-ID"));
    assert!(!labels.contains("tag_my-tag"));
}

#[test]
fn prometheus_config_non_enum_labels_are_the_seven_service_labels() {
    let expected = [
        "guardrail_name",
        "status",
        "error_type",
        "hook_type",
        "purpose",
        "file_type",
        "result",
    ];
    assert_eq!(NON_ENUM_METRIC_LABELS, expected);
    let distinct: BTreeSet<&str> = NON_ENUM_METRIC_LABELS.into_iter().collect();
    assert_eq!(distinct.len(), NON_ENUM_METRIC_LABELS.len());
}

#[test]
fn prometheus_config_pretty_prints_validation_errors() {
    let results = ValidationResults {
        metric_errors: vec![MetricValidationError {
            metric_name: String::from("bad_metric"),
            valid_metrics: Vec::new(),
        }],
        label_errors: vec![LabelValidationError {
            metric_name: String::from("litellm_spend_metric"),
            invalid_labels: vec![String::from("bogus")],
            valid_labels: vec![String::from("model")],
            caller_identity_mode: String::from("api_key_alias"),
        }],
    };
    let report = pretty_print_validation_errors(&results);
    assert!(report.contains("Invalid metric name: bad_metric"));
    assert!(report.contains("Invalid labels for metric 'litellm_spend_metric': [\"bogus\"]"));
}

#[test]
fn prometheus_config_pretty_prints_enabled_metrics_and_filters() {
    let mut factory = CollectorFactory::new(schema());
    let configs = [config(&["litellm_spend_metric"], &["model", "team"])];
    factory.parse_prometheus_config(&configs).unwrap();
    let report = pretty_print_prometheus_config(&factory.enabled_metrics, &factory.label_filters);
    assert!(report.contains("litellm_spend_metric"));
    assert!(report.contains("model"));
    assert!(report.contains("team"));
}
