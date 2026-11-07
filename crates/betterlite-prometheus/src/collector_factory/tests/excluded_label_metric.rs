#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::{
    Collector as _, ExcludedLabelMetric, MetricUpdate, MetricValue,
};
use betterlite_prometheus::collector_factory::config::MetricSet;
use rust_decimal::Decimal;

use crate::support::FakeMetric;

fn set(names: &[&str]) -> MetricSet {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn wrapper(excluded: &[&str]) -> ExcludedLabelMetric<FakeMetric> {
    ExcludedLabelMetric {
        metric: FakeMetric::registered("fake", &[]),
        excluded_labels: set(excluded),
    }
}

#[test]
fn excluded_label_metric_drops_excluded_labels() {
    let wrapped = wrapper(&["team"]);
    let labeled = wrapped.labels(&[("model", "gpt-4"), ("team", "alpha")]);
    assert_eq!(
        labeled.metric.labels,
        Vec::from([(String::from("model"), String::from("gpt-4"))])
    );
}

#[test]
fn excluded_label_metric_keeps_kept_labels_in_order() {
    let wrapped = wrapper(&["team", "end_user"]);
    let labeled = wrapped.labels(&[
        ("model", "gpt-4"),
        ("team", "alpha"),
        ("user", "max"),
        ("end_user", "eu-1"),
    ]);
    assert_eq!(
        labeled.metric.labels,
        Vec::from([
            (String::from("model"), String::from("gpt-4")),
            (String::from("user"), String::from("max"))
        ])
    );
}

#[test]
fn excluded_label_metric_returns_the_metric_when_no_label_remains() {
    let wrapped = wrapper(&["team"]);
    let labeled = wrapped.labels(&[("team", "alpha")]);
    assert!(labeled.metric.labels.is_empty());
    assert_eq!(wrapped.metric.label_calls(), 0);
}

#[test]
fn excluded_label_metric_forwards_every_update() {
    let wrapped = wrapper(&["team"]);
    let value = MetricValue::Finite(Decimal::new(150, 2));
    wrapped.inc(value);
    wrapped.set(value);
    wrapped.observe(value);
    wrapped.remove();
    let updates = [
        MetricUpdate::Inc(value),
        MetricUpdate::Set(value),
        MetricUpdate::Observe(value),
        MetricUpdate::Remove,
    ];
    let expected = updates
        .iter()
        .map(|update| format!("fake:{update:?}"))
        .collect::<Vec<_>>();
    assert_eq!(wrapped.metric.entries(), expected);
}
