use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use crate::multiprocess::writer::MultiProcessWriter;

use gauge_mode::GaugeMode;
use sample::{Sample, added};

mod binding;
pub mod gauge_mode;
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
            other @ (Bucket::Seconds(_) | Bucket::Infinite) => *other,
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
    pub multiprocess_mode: GaugeMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeriesRemoval {
    Removed,
    Absent,
    Unsupported,
}

// one registered collector: the description, the label set of this handle, the samples clones share and when it was created
#[derive(Clone, Debug)]
pub struct PrometheusMetric {
    pub description: Arc<CollectorDescription>,
    pub label_values: Vec<String>,
    pub samples: Arc<Mutex<IndexMap<Vec<String>, Sample>>>,
    pub created: SystemTime,
    pub multiprocess: Arc<Mutex<Option<Arc<MultiProcessWriter>>>>,
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
        let registered_name = match kind {
            CollectorKind::Counter => name.strip_suffix("_total").unwrap_or(name),
            CollectorKind::Gauge | CollectorKind::Histogram => name,
        };
        let created = SystemTime::now();
        let description = CollectorDescription {
            name: registered_name.to_owned(),
            kind,
            help: help.to_owned(),
            label_names: label_names
                .iter()
                .map(|label| (*label).to_owned())
                .collect(),
            buckets,
            multiprocess_mode: GaugeMode::All,
        };
        let mut samples = IndexMap::new();
        // prometheus_client builds the one series of a label-less metric in the constructor, so it is written before any update
        if description.label_names.is_empty() {
            let sample = Sample {
                total: MetricValue::Finite(Decimal::ZERO),
                count: 0,
                buckets: vec![0; description.buckets.len()],
                created,
            };
            let _inserted = samples.insert(Vec::new(), sample);
        }
        Self {
            description: Arc::new(description),
            label_values: Vec::new(),
            samples: Arc::new(Mutex::new(samples)),
            created,
            multiprocess: Arc::new(Mutex::new(None)),
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
        self.warn_removal();
        if label_values.is_empty() || self.description.label_names.len() != label_values.len() {
            return SeriesRemoval::Unsupported;
        }
        let removed = self
            .samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shift_remove(&label_key(label_values));
        if removed.is_some() {
            SeriesRemoval::Removed
        } else {
            SeriesRemoval::Absent
        }
    }

    #[must_use]
    pub fn series(&self, label_values: &[Option<String>]) -> Option<Sample> {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&label_key(label_values))
            .cloned()
    }

    // prometheus_client writes a family's series in the order their label sets were first used, its dict's order
    #[must_use]
    pub fn series_in_use_order(&self) -> Vec<(Vec<String>, Sample)> {
        let samples = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        let mut series = Vec::with_capacity(samples.len());
        for (labels, sample) in &*samples {
            series.push((labels.clone(), sample.clone()));
        }
        drop(samples);
        series
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
            created: self.created,
            multiprocess: Arc::clone(&self.multiprocess),
        }
    }

    fn update(&self, update: MetricUpdate) {
        let mut samples = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        if update == MetricUpdate::Remove {
            let _removed = samples.shift_remove(&self.label_values);
            self.warn_removal();
            return;
        }
        let bounds = self.description.buckets.as_slice();
        // a label-less metric's series is the constructor's, with the metric's own creation time; a child stamps now
        let created = if self.description.label_names.is_empty() {
            self.created
        } else {
            SystemTime::now()
        };
        let sample = samples
            .entry(self.label_values.clone())
            .or_insert_with(|| Sample {
                total: MetricValue::Finite(Decimal::ZERO),
                count: 0,
                buckets: vec![0; bounds.len()],
                created,
            });
        match (self.description.kind, update) {
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Inc(value)) => {
                sample.total = added(sample.total, value);
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Set(value)) => {
                sample.total = value;
            }
            (CollectorKind::Histogram, MetricUpdate::Observe(value)) => {
                sample.observe(value, bounds);
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Observe(_))
            | (CollectorKind::Histogram, MetricUpdate::Inc(_) | MetricUpdate::Set(_))
            | (
                CollectorKind::Counter | CollectorKind::Gauge | CollectorKind::Histogram,
                MetricUpdate::Remove,
            ) => {}
        }
        // the bound writer takes the update before the samples lock lets the next one in, so both see the same order
        self.write_update(update);
        drop(samples);
    }
}
