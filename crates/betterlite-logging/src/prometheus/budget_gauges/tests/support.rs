#![cfg(test)]

use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use crate::prometheus_helpers::bounded_series_tracker::{ChildRemoval, RemovableMetric};
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;
use crate::prometheus_label_schema::PrometheusLabelSettings;

#[derive(Debug, thiserror::Error)]
#[error("the probe gauge refused the removal")]
pub(crate) struct ProbeError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProbeEvent {
    Set {
        metric: String,
        labels: Vec<(String, String)>,
        value: MetricValue,
    },
    Removal {
        metric: String,
        labels: Vec<Option<String>>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ProbeGauge {
    metric: String,
    labels: Vec<(String, String)>,
    log: Arc<Mutex<Vec<ProbeEvent>>>,
    removal: ChildRemoval,
}

impl Collector for ProbeGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut child = self.clone();
        child.labels = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        child
    }

    fn update(&self, update: MetricUpdate) {
        if let MetricUpdate::Set(value) = update {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ProbeEvent::Set {
                    metric: self.metric.clone(),
                    labels: self.labels.clone(),
                    value,
                });
        }
    }
}

impl RemovableMetric for ProbeGauge {
    type Error = ProbeError;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProbeEvent::Removal {
                metric: self.metric.clone(),
                labels: label_values.to_vec(),
            });
        Ok(self.removal)
    }
}

pub(crate) type ProbeLog = Arc<Mutex<Vec<ProbeEvent>>>;

pub(crate) fn new_log() -> ProbeLog {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn field(
    metric: &'static str,
    labels: &[&str],
    log: &ProbeLog,
) -> GaugeField<ProbeGauge> {
    GaugeField {
        metric_name: metric,
        supported_labels: words(labels),
        collector: ProbeGauge {
            metric: metric.to_owned(),
            labels: Vec::new(),
            log: Arc::clone(log),
            removal: ChildRemoval::Removed,
        },
    }
}

pub(crate) fn rig(
    remaining: &'static str,
    max: &'static str,
    hours: &'static str,
    labels: &[&str],
) -> (EntityBudgetGauges<ProbeGauge>, ProbeLog) {
    let log = new_log();
    let gauges = EntityBudgetGauges {
        remaining: field(remaining, labels, &log),
        max: field(max, labels, &log),
        remaining_hours: field(hours, labels, &log),
    };
    (gauges, log)
}

pub(crate) fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

pub(crate) fn events(log: &ProbeLog) -> Vec<ProbeEvent> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(crate) fn finite(text: &str) -> MetricValue {
    MetricValue::Finite(decimal(text))
}

pub(crate) fn hours_value(hours: f64) -> MetricValue {
    MetricValue::Finite(Decimal::from_f64(hours).unwrap())
}

pub(crate) fn set(metric: &str, labels: &[(&str, &str)], value: MetricValue) -> ProbeEvent {
    ProbeEvent::Set {
        metric: metric.to_owned(),
        labels: labels
            .iter()
            .map(|(name, label_value)| ((*name).to_owned(), (*label_value).to_owned()))
            .collect(),
        value,
    }
}

pub(crate) fn removal(metric: &str, labels: &[Option<&str>]) -> ProbeEvent {
    let mut child_labels = Vec::with_capacity(labels.len());
    for value in labels {
        child_labels.push(value.map(str::to_owned));
    }
    ProbeEvent::Removal {
        metric: metric.to_owned(),
        labels: child_labels,
    }
}

pub(crate) const fn source(
    settings: &PrometheusLabelSettings,
    tracking: EndUserCostTrackingSettings,
) -> LabelSource<'_> {
    LabelSource::Values {
        settings,
        end_user_settings: tracking,
    }
}

pub(crate) const fn prometheus_only() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(true),
    }
}
