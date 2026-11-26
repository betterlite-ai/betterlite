use std::collections::BTreeMap;

use crate::collector_factory::collector::{MetricUpdate, MetricValue};
use crate::collector_registry::metric::sample::{value_of, within};
use crate::collector_registry::metric::{Bucket, CollectorDescription, CollectorKind};
use crate::multiprocess::MultiprocessError;
use crate::scrape_endpoint::float::float_to_go_string;

use super::file::{FileState, ValueChange};

pub(crate) enum SeriesKeys {
    Single(String),
    Histogram(HistogramKeys),
}

pub(crate) struct HistogramKeys {
    sum: String,
    buckets: Vec<BucketKey>,
}

struct BucketKey {
    key: String,
    bound: Bucket,
}

impl SeriesKeys {
    // the keys a new series creates, in prometheus_client's order: a histogram's x_sum comes before its buckets
    pub(crate) fn created(&self) -> Vec<&str> {
        match self {
            Self::Single(key) => vec![key.as_str()],
            Self::Histogram(keys) => {
                let mut created = Vec::with_capacity(keys.buckets.len().saturating_add(1));
                created.push(keys.sum.as_str());
                created.extend(keys.buckets.iter().map(|bucket| bucket.key.as_str()));
                created
            }
        }
    }
}

// mmap_key: [metric name, sample name, the labels object, help]; a histogram bucket carries le as one more label
pub(crate) fn keys(
    description: &CollectorDescription,
    label_values: &[String],
) -> Result<SeriesKeys, MultiprocessError> {
    match description.kind {
        CollectorKind::Counter => {
            let sample = format!("{}_total", description.name);
            let key = key_of(description, &sample, label_values, None)?;
            Ok(SeriesKeys::Single(key))
        }
        CollectorKind::Gauge => {
            let key = key_of(description, &description.name, label_values, None)?;
            Ok(SeriesKeys::Single(key))
        }
        CollectorKind::Histogram => {
            let histogram = histogram_keys(description, label_values)?;
            Ok(SeriesKeys::Histogram(histogram))
        }
    }
}

fn histogram_keys(
    description: &CollectorDescription,
    label_values: &[String],
) -> Result<HistogramKeys, MultiprocessError> {
    let sample = format!("{}_sum", description.name);
    let sum = key_of(description, &sample, label_values, None)?;
    let mut buckets = Vec::with_capacity(description.buckets.len());
    let bucket_name = format!("{}_bucket", description.name);
    for bound in &description.buckets {
        let le = float_to_go_string(bound.key());
        let key = key_of(description, &bucket_name, label_values, Some(&le))?;
        buckets.push(BucketKey { key, bound: *bound });
    }
    Ok(HistogramKeys { sum, buckets })
}

fn key_of(
    description: &CollectorDescription,
    sample: &str,
    label_values: &[String],
    le: Option<&str>,
) -> Result<String, MultiprocessError> {
    let mut labels: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, value) in description.label_names.iter().zip(label_values) {
        let _inserted = labels.insert(name.as_str(), value.as_str());
    }
    if let Some(bound) = le {
        let _inserted = labels.insert("le", bound);
    }
    serde_json::to_string(&(
        description.name.as_str(),
        sample,
        labels,
        description.help.as_str(),
    ))
    .map_err(|source| MultiprocessError::Key { source })
}

pub(crate) fn apply(
    file: &mut FileState,
    keys: &SeriesKeys,
    update: MetricUpdate,
) -> Result<(), MultiprocessError> {
    for key in keys.created() {
        file.ensure(key)?;
    }
    match (keys, update) {
        (SeriesKeys::Single(key), MetricUpdate::Inc(amount)) => {
            file.change(key, ValueChange::Add(value_of(amount)))
        }
        (SeriesKeys::Single(key), MetricUpdate::Set(value)) => {
            file.change(key, ValueChange::Replace(value_of(value)))
        }
        (SeriesKeys::Histogram(histogram), MetricUpdate::Observe(amount)) => {
            observe(file, histogram, amount)
        }
        (SeriesKeys::Single(_), MetricUpdate::Observe(_) | MetricUpdate::Remove)
        | (
            SeriesKeys::Histogram(_),
            MetricUpdate::Inc(_) | MetricUpdate::Set(_) | MetricUpdate::Remove,
        ) => Ok(()),
    }
}

// Histogram.observe: the sum takes every amount, and the first bound the amount does not exceed takes one more
fn observe(
    file: &mut FileState,
    keys: &HistogramKeys,
    amount: MetricValue,
) -> Result<(), MultiprocessError> {
    file.change(&keys.sum, ValueChange::Add(value_of(amount)))?;
    let landed = keys
        .buckets
        .iter()
        .find(|bucket| within(amount, bucket.bound));
    if let Some(bucket) = landed {
        file.change(&bucket.key, ValueChange::Add(1.0_f64))?;
    }
    Ok(())
}
