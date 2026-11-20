use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use rust_decimal::Decimal;

use crate::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

use sample::{Sample, added, within};

pub mod sample;

const MISSING_LABEL: &str = "None";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectorKind {
    Counter,
    Gauge,
    Histogram,
}

impl CollectorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

// prometheus_client keeps upper bounds as floats, so a custom list keeps negatives and NaN reaches prepare
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bucket {
    Seconds(f64),
    Infinite,
}

impl Bucket {
    #[must_use]
    pub const fn key(self) -> f64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::Infinite => f64::INFINITY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BucketError {
    #[error("Buckets not in sorted order")]
    NotSorted,
    #[error("Must have at least two buckets")]
    TooFew,
}

// Histogram._prepare_buckets: a +Inf upper bound is the same infinity of the last bucket, then comes too few
pub fn prepare_buckets(source: &[Bucket]) -> Result<Vec<Bucket>, BucketError> {
    let mut prepared: Vec<Bucket> = source
        .iter()
        .map(|bound| match bound {
            Bucket::Seconds(seconds) if seconds.is_infinite() && seconds.is_sign_positive() => {
                Bucket::Infinite
            }
            other => *other,
        })
        .collect();
    let unordered = prepared
        .iter()
        .zip(prepared.iter().skip(1))
        .any(|(left, right)| right.key() < left.key());
    let not_a_number = prepared
        .iter()
        .any(|bound| matches!(bound, Bucket::Seconds(seconds) if seconds.is_nan()));
    if unordered || not_a_number {
        return Err(BucketError::NotSorted);
    }
    if !matches!(prepared.last(), Some(Bucket::Infinite)) {
        prepared.push(Bucket::Infinite);
    }
    if prepared.len() < 2 {
        return Err(BucketError::TooFew);
    }
    Ok(prepared)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollectorDescription {
    pub name: String,
    pub kind: CollectorKind,
    pub help: String,
    pub label_names: Vec<String>,
    pub buckets: Vec<Bucket>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeriesRemoval {
    Removed,
    Absent,
    Unsupported,
}

// one registered collector: the description, the label set of this handle and the samples clones share
#[derive(Clone, Debug)]
pub struct PrometheusMetric {
    pub description: Arc<CollectorDescription>,
    label_values: Vec<String>,
    samples: Arc<Mutex<BTreeMap<Vec<String>, Sample>>>,
}

impl PrometheusMetric {
    // prometheus_client strips _total from a counter name; a histogram takes what prepare_buckets validated
    #[must_use]
    pub fn new(
        kind: CollectorKind,
        name: &str,
        help: &str,
        label_names: &[&str],
        buckets: Vec<Bucket>,
    ) -> Self {
        let name = match kind {
            CollectorKind::Counter => name.strip_suffix("_total").unwrap_or(name),
            CollectorKind::Gauge | CollectorKind::Histogram => name,
        };
        let description = CollectorDescription {
            name: name.to_owned(),
            kind,
            help: help.to_owned(),
            label_names: label_names
                .iter()
                .map(|label| (*label).to_owned())
                .collect(),
            buckets,
        };
        Self {
            description: Arc::new(description),
            label_values: Vec::new(),
            samples: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    // CollectorRegistry._get_names: a counter also owns _total and _created, a histogram its four suffixes
    #[must_use]
    pub fn sample_names(&self) -> Vec<String> {
        let name = self.description.name.as_str();
        match self.description.kind {
            CollectorKind::Counter => vec![
                name.to_owned(),
                format!("{name}_total"),
                format!("{name}_created"),
            ],
            CollectorKind::Gauge => vec![name.to_owned()],
            CollectorKind::Histogram => vec![
                name.to_owned(),
                format!("{name}_bucket"),
                format!("{name}_sum"),
                format!("{name}_count"),
                format!("{name}_created"),
            ],
        }
    }

    // MetricWrapperBase.remove: no label names or the wrong count raises ValueError; an absent child is gone
    #[must_use]
    pub fn remove_series(&self, label_values: &[Option<String>]) -> SeriesRemoval {
        if label_values.is_empty() || self.description.label_names.len() != label_values.len() {
            return SeriesRemoval::Unsupported;
        }
        let mut samples = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        if samples.remove(&label_key(label_values)).is_some() {
            return SeriesRemoval::Removed;
        }
        SeriesRemoval::Absent
    }

    #[must_use]
    pub fn series(&self, label_values: &[Option<String>]) -> Option<Sample> {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&label_key(label_values))
            .cloned()
    }
}

fn label_key(label_values: &[Option<String>]) -> Vec<String> {
    label_values
        .iter()
        .map(|value| value.clone().unwrap_or_else(|| MISSING_LABEL.to_owned()))
        .collect()
}

impl Collector for PrometheusMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let values: Vec<String> = self
            .description
            .label_names
            .iter()
            .map(|name| {
                labels
                    .iter()
                    .find(|(label, _)| *label == name.as_str())
                    .map_or_else(
                        || MISSING_LABEL.to_owned(),
                        |(_, value)| (*value).to_owned(),
                    )
            })
            .collect();
        Self {
            description: Arc::clone(&self.description),
            label_values: values,
            samples: Arc::clone(&self.samples),
        }
    }

    fn update(&self, update: MetricUpdate) {
        let mut samples = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        if let MetricUpdate::Remove = update {
            samples.remove(&self.label_values);
            return;
        }
        let bounds = self.description.buckets.as_slice();
        let sample = samples
            .entry(self.label_values.clone())
            .or_insert_with(|| Sample {
                total: MetricValue::Finite(Decimal::ZERO),
                count: 0,
                buckets: vec![0; bounds.len()],
            });
        match (self.description.kind, update) {
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Inc(value)) => {
                sample.total = added(sample.total, value);
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Set(value)) => {
                sample.total = value;
            }
            (CollectorKind::Histogram, MetricUpdate::Observe(value)) => {
                sample.total = added(sample.total, value);
                sample.count = sample.count.saturating_add(1);
                for (index, bound) in bounds.iter().enumerate() {
                    if within(value, *bound)
                        && let Some(entry) = sample.buckets.get_mut(index)
                    {
                        *entry = entry.saturating_add(1);
                    }
                }
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Observe(_))
            | (CollectorKind::Histogram, MetricUpdate::Inc(_) | MetricUpdate::Set(_))
            | (
                CollectorKind::Counter | CollectorKind::Gauge | CollectorKind::Histogram,
                MetricUpdate::Remove,
            ) => {}
        }
    }
}
