#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_prometheus::collector_factory::collector::{
    Collector as _, MetricLike, MetricUpdate, MetricValue,
};
use betterlite_prometheus::collector_factory::config::{MetricSet, PrometheusMetricsConfig};
use betterlite_prometheus::collector_factory::error::CollectorFactoryError;
use betterlite_prometheus::collector_factory::factory::{CollectorFactory, is_metric_enabled};
use rstest::rstest;
use rust_decimal::Decimal;

use crate::support::{ChangingSchema, FakeMetric, FakeSchema, FakeSchemaError};

fn schema() -> FakeSchema {
    FakeSchema::new(&[
        ("litellm_spend_metric", &["model", "team", "end_user"]),
        ("litellm_requests_metric", &["model", "team"]),
    ])
}

fn config(metric: &str, labels: &[&str]) -> PrometheusMetricsConfig {
    PrometheusMetricsConfig {
        group: String::from("test"),
        metrics: vec![metric.to_owned()],
        include_labels: labels.iter().map(|label| (*label).to_owned()).collect(),
    }
}

fn set(names: &[&str]) -> MetricSet {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[rstest]
#[case::excluded(MetricSet::new(), set(&["litellm_spend_metric"]), false)]
#[case::excluded_even_when_enabled(set(&["litellm_spend_metric"]), set(&["litellm_spend_metric"]), false)]
#[case::empty_enable_list(MetricSet::new(), MetricSet::new(), true)]
#[case::listed(set(&["litellm_spend_metric"]), MetricSet::new(), true)]
#[case::unlisted(set(&["litellm_requests_metric"]), MetricSet::new(), false)]
fn prometheus_factory_is_metric_enabled(
    #[case] enabled: MetricSet,
    #[case] excluded: MetricSet,
    #[case] expected: bool,
) {
    assert_eq!(
        is_metric_enabled(&enabled, &excluded, "litellm_spend_metric"),
        expected
    );
}

#[test]
fn prometheus_factory_disabled_metric_returns_no_op() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(&[String::from("litellm_spend_metric")], &[], &[], &[])
        .unwrap();
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model"]);
    assert!(matches!(metric, MetricLike::NoOp(_)));
    metric.inc(MetricValue::Finite(Decimal::ONE));
    metric.observe(MetricValue::PositiveInfinity);
    metric.set(MetricValue::NegativeInfinity);
    metric.remove();
}

#[test]
fn prometheus_factory_disables_metrics_outside_the_enable_list() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_prometheus_config(&[config("litellm_requests_metric", &[])])
        .unwrap();
    let build = factory.create_metric_factory(FakeMetric::registered);
    assert!(matches!(
        build("litellm_spend_metric", &["model"]),
        MetricLike::NoOp(_)
    ));
    assert!(matches!(
        build("litellm_requests_metric", &["model"]),
        MetricLike::Real(_)
    ));
}

#[test]
fn prometheus_factory_builds_the_real_metric() {
    let factory = CollectorFactory::new(schema());
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model", "team"]);
    let MetricLike::Real(real) = metric else {
        panic!("expected the real metric");
    };
    assert_eq!(real.name, "litellm_spend_metric");
    assert_eq!(real.labelnames, Vec::from(["model", "team"]));
}

#[test]
fn prometheus_factory_wraps_the_metric_when_labels_are_excluded() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(&[], &[String::from("team")], &[], &[])
        .unwrap();
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model", "team"]);
    let MetricLike::Excluded(wrapper) = metric else {
        panic!("expected the excluded-label wrapper");
    };
    assert_eq!(wrapper.metric.labelnames, Vec::from(["model"]));
    assert!(wrapper.excluded_labels.contains("team"));
}

#[test]
fn prometheus_factory_keeps_the_metric_when_no_registered_label_is_excluded() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(&[], &[String::from("end_user")], &[], &[])
        .unwrap();
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model", "team"]);
    assert!(matches!(metric, MetricLike::Real(_)));
}

#[test]
fn prometheus_factory_get_labels_applies_exclusions_and_filters() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(&[], &[String::from("team")], &[], &[])
        .unwrap();
    factory
        .parse_prometheus_config(&[config("litellm_spend_metric", &["model", "team"])])
        .unwrap();
    assert_eq!(
        factory
            .get_labels_for_metric("litellm_spend_metric")
            .unwrap(),
        Vec::from(["model"])
    );
}

#[test]
fn prometheus_factory_get_labels_without_filters_keeps_the_schema() {
    let mut factory = CollectorFactory::new(schema());
    assert_eq!(
        factory
            .get_labels_for_metric("litellm_requests_metric")
            .unwrap(),
        Vec::from(["model", "team"])
    );
}

#[test]
fn prometheus_factory_get_labels_is_cached_for_the_instance() {
    let schema = ChangingSchema {
        responses: Arc::new(Mutex::new(VecDeque::from([
            Vec::from([String::from("model")]),
            Vec::from([String::from("team")]),
        ]))),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let calls = Arc::clone(&schema.calls);
    let mut factory = CollectorFactory::new(schema);
    let first = factory
        .get_labels_for_metric("litellm_spend_metric")
        .unwrap();
    let second = factory
        .get_labels_for_metric("litellm_spend_metric")
        .unwrap();
    assert_eq!(first, Vec::from(["model"]));
    assert_eq!(second, Vec::from(["model"]));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn prometheus_factory_reports_an_unknown_schema_as_an_error() {
    let mut factory = CollectorFactory::new(FakeSchema::new(&[]));
    let error = factory
        .get_labels_for_metric("litellm_spend_metric")
        .unwrap_err();
    assert!(matches!(
        error,
        CollectorFactoryError::LabelSchema(FakeSchemaError::Missing { .. })
    ));
}

#[test]
fn prometheus_factory_metric_values_keep_decimal_precision() {
    let exact = MetricValue::Finite(Decimal::new(1234, 2));
    assert_eq!(exact, MetricValue::Finite(Decimal::new(1234, 2)));
    assert_ne!(exact, MetricValue::Finite(Decimal::new(1235, 2)));
    assert_ne!(MetricValue::PositiveInfinity, MetricValue::NegativeInfinity);
    assert_ne!(MetricValue::PositiveInfinity, MetricValue::NotANumber);
    assert_ne!(MetricValue::NegativeInfinity, MetricValue::NotANumber);
    assert_ne!(MetricValue::NotANumber, exact);
}

#[test]
fn prometheus_factory_forwards_operations_on_the_real_metric() {
    let factory = CollectorFactory::new(schema());
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model", "team"]);
    let labeled = metric.labels(&[("model", "gpt-4")]);
    labeled.inc(MetricValue::Finite(Decimal::new(150, 2)));
    labeled.set(MetricValue::PositiveInfinity);
    labeled.observe(MetricValue::NegativeInfinity);
    labeled.observe(MetricValue::NotANumber);
    labeled.remove();
    let expected = [
        MetricUpdate::Inc(MetricValue::Finite(Decimal::new(150, 2))),
        MetricUpdate::Set(MetricValue::PositiveInfinity),
        MetricUpdate::Observe(MetricValue::NegativeInfinity),
        MetricUpdate::Observe(MetricValue::NotANumber),
        MetricUpdate::Remove,
    ]
    .iter()
    .map(|update| format!("litellm_spend_metric:{update:?}"))
    .collect::<Vec<_>>();
    let MetricLike::Real(real) = &labeled else {
        panic!("expected the real metric");
    };
    assert_eq!(
        real.labels,
        Vec::from([(String::from("model"), String::from("gpt-4"))])
    );
    assert_eq!(real.entries(), expected);
}

#[test]
fn prometheus_factory_forwards_operations_on_the_excluded_metric() {
    let mut factory = CollectorFactory::new(schema());
    factory
        .parse_exclude_config(&[], &[String::from("team")], &[], &[])
        .unwrap();
    let build = factory.create_metric_factory(FakeMetric::registered);
    let metric = build("litellm_spend_metric", &["model", "team"]);
    let labeled = metric.labels(&[("model", "gpt-4"), ("team", "alpha")]);
    labeled.inc(MetricValue::Finite(Decimal::new(150, 2)));
    labeled.set(MetricValue::PositiveInfinity);
    labeled.observe(MetricValue::NegativeInfinity);
    labeled.observe(MetricValue::NotANumber);
    labeled.remove();
    let expected = [
        MetricUpdate::Inc(MetricValue::Finite(Decimal::new(150, 2))),
        MetricUpdate::Set(MetricValue::PositiveInfinity),
        MetricUpdate::Observe(MetricValue::NegativeInfinity),
        MetricUpdate::Observe(MetricValue::NotANumber),
        MetricUpdate::Remove,
    ]
    .iter()
    .map(|update| format!("litellm_spend_metric:{update:?}"))
    .collect::<Vec<_>>();
    let MetricLike::Excluded(wrapper) = &labeled else {
        panic!("expected the excluded-label wrapper");
    };
    assert_eq!(
        wrapper.metric.labels,
        Vec::from([(String::from("model"), String::from("gpt-4"))])
    );
    assert_eq!(wrapper.metric.entries(), expected);
}
