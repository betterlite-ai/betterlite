#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_prometheus::collector_factory::collector::{ExcludedLabelMetric, MetricLike};
use betterlite_prometheus::no_op_metric::NoOpMetric;

use super::{budget_gauges_are_noop, customer_budget_metrics_enabled};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use crate::prometheus::budget_gauges::test_support::{ProbeGauge, field, new_log, words};

fn gate(disable: Option<bool>, enable: Option<bool>) -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: disable,
        enable_prometheus_only: enable,
    }
}

fn noop_field(metric: &'static str) -> GaugeField<MetricLike<ProbeGauge>> {
    GaugeField {
        metric_name: metric,
        supported_labels: words(&["end_user"]),
        collector: MetricLike::NoOp(NoOpMetric),
    }
}

fn noop_gauges() -> EntityBudgetGauges<MetricLike<ProbeGauge>> {
    EntityBudgetGauges {
        remaining: noop_field("litellm_remaining_customer_budget_metric"),
        max: noop_field("litellm_customer_max_budget_metric"),
        remaining_hours: noop_field("litellm_customer_budget_remaining_hours_metric"),
    }
}

#[test]
fn both_flags_unset_leave_the_gate_closed() {
    assert!(!customer_budget_metrics_enabled(&gate(None, None)));
}

#[test]
fn the_exact_true_opt_in_opens_the_gate() {
    assert!(customer_budget_metrics_enabled(&gate(None, Some(true))));
}

#[test]
fn a_false_opt_in_keeps_the_gate_closed() {
    assert!(!customer_budget_metrics_enabled(&gate(None, Some(false))));
}

#[test]
fn the_disable_flag_closes_an_open_gate() {
    assert!(!customer_budget_metrics_enabled(&gate(
        Some(true),
        Some(true)
    )));
}

#[test]
fn three_noop_customer_gauges_are_noop() {
    assert!(budget_gauges_are_noop(&noop_gauges()));
}

#[test]
fn one_real_customer_gauge_is_not_noop() {
    let log = new_log();
    let mut gauges = noop_gauges();
    gauges.remaining_hours = field(
        "litellm_customer_budget_remaining_hours_metric",
        &["end_user"],
        &log,
    );
    assert!(!budget_gauges_are_noop(&gauges));
}

#[test]
fn one_excluded_customer_gauge_is_not_noop() {
    let mut gauges = noop_gauges();
    gauges.max = GaugeField {
        metric_name: "litellm_customer_max_budget_metric",
        supported_labels: words(&["end_user"]),
        collector: MetricLike::Excluded(ExcludedLabelMetric {
            metric: field(
                "litellm_customer_max_budget_metric",
                &["end_user"],
                &new_log(),
            )
            .collector,
            excluded_labels: BTreeSet::new(),
        }),
    };
    assert!(!budget_gauges_are_noop(&gauges));
}
