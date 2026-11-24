use std::cmp::Ordering;
use std::ops::Add;

use indexmap::IndexMap;

use crate::multiprocess::MultiprocessError;
use crate::scrape_endpoint::float::float_to_go_string;
use crate::scrape_endpoint::{MetricKind, MetricSample};

use super::{FamilyBuilder, GaugeMode, RawSample, SampleKey};

type Samples = IndexMap<SampleKey, f64>;
type Buckets = IndexMap<Vec<(String, String)>, Vec<(f64, f64)>>;

// _accumulate_metrics for one family: counters and summaries sum, gauges combine by their mode, histograms count bounds
pub(super) fn samples(
    name: &str,
    family: &FamilyBuilder,
) -> Result<Vec<MetricSample>, MultiprocessError> {
    match family.kind {
        MetricKind::Gauge => Ok(gauge(family)),
        MetricKind::Histogram => histogram(name, family),
        MetricKind::Counter | MetricKind::Summary => Ok(sum(&family.samples)),
    }
}

fn sum(raw: &[RawSample]) -> Vec<MetricSample> {
    let mut summed = Samples::new();
    for sample in raw {
        let key = SampleKey {
            name: sample.name.clone(),
            labels: sample.labels.clone(),
        };
        accumulate_into(&mut summed, key, sample.value);
    }
    collected(summed)
}

fn accumulate_into(samples: &mut Samples, key: SampleKey, value: f64) {
    let total = samples.entry(key).or_insert(0.0_f64);
    let sum = Add::add(*total, value);
    *total = sum;
}

fn collected(samples: Samples) -> Vec<MetricSample> {
    let mut output = Vec::with_capacity(samples.len());
    for (key, value) in samples {
        output.push(MetricSample {
            name: key.name,
            labels: key.labels,
            value,
        });
    }
    output
}

fn gauge(family: &FamilyBuilder) -> Vec<MetricSample> {
    let mode = family.mode.unwrap_or(GaugeMode::All);
    let mut samples = Samples::new();
    let mut timestamps = Samples::new();
    for sample in &family.samples {
        gauge_sample(sample, mode, &mut samples, &mut timestamps);
    }
    collected(samples)
}

fn gauge_sample(raw: &RawSample, mode: GaugeMode, samples: &mut Samples, timestamps: &mut Samples) {
    let labels = if mode.combines_without_pid() {
        without_label(&raw.labels, "pid")
    } else {
        raw.labels.clone()
    };
    let key = SampleKey {
        name: raw.name.clone(),
        labels,
    };
    match mode {
        GaugeMode::Min => keep_smallest(samples, key, raw.value),
        GaugeMode::Max => keep_largest(samples, key, raw.value),
        GaugeMode::Sum => accumulate_into(samples, key, raw.value),
        GaugeMode::MostRecent => keep_most_recent(samples, timestamps, key, raw),
        GaugeMode::All => {
            let _previous = samples.insert(key, raw.value);
        }
    }
}

fn without_label(labels: &[(String, String)], excluded: &str) -> Vec<(String, String)> {
    let mut kept = Vec::with_capacity(labels.len());
    for (name, value) in labels {
        if name != excluded {
            kept.push((name.clone(), value.clone()));
        }
    }
    kept
}

fn keep_smallest(samples: &mut Samples, key: SampleKey, value: f64) {
    let current = samples.get(&key).copied();
    if current.is_none_or(|existing| value < existing) {
        let _inserted = samples.insert(key, value);
    }
}

fn keep_largest(samples: &mut Samples, key: SampleKey, value: f64) {
    let current = samples.get(&key).copied();
    if current.is_none_or(|existing| value > existing) {
        let _inserted = samples.insert(key, value);
    }
}

// the value whose timestamp is strictly greater than the best so far, which starts at 0
fn keep_most_recent(
    samples: &mut Samples,
    timestamps: &mut Samples,
    key: SampleKey,
    sample: &RawSample,
) {
    let timestamp = sample.timestamp.unwrap_or(0.0_f64);
    let current = timestamps.get(&key).copied().unwrap_or(0.0_f64);
    if current < timestamp {
        let _replaced = samples.insert(key.clone(), sample.value);
        let _stamped = timestamps.insert(key, timestamp);
    }
}

fn histogram(name: &str, family: &FamilyBuilder) -> Result<Vec<MetricSample>, MultiprocessError> {
    let mut samples = Samples::new();
    let mut buckets = Buckets::new();
    for sample in &family.samples {
        histogram_sample(sample, &mut samples, &mut buckets)?;
    }
    add_buckets(name, &mut samples, buckets);
    Ok(collected(samples))
}

// a sample with an le label is one bucket's count; every other sample, the _sum, is summed like a counter
fn histogram_sample(
    sample: &RawSample,
    samples: &mut Samples,
    buckets: &mut Buckets,
) -> Result<(), MultiprocessError> {
    let Some((_label, bound_text)) = sample
        .labels
        .iter()
        .find(|(label, _value)| label.as_str() == "le")
    else {
        let key = SampleKey {
            name: sample.name.clone(),
            labels: sample.labels.clone(),
        };
        accumulate_into(samples, key, sample.value);
        return Ok(());
    };
    let Ok(bound) = bound_text.as_str().parse::<f64>() else {
        return Err(MultiprocessError::InvalidBucketBound {
            bound: bound_text.clone(),
        });
    };
    let labels = without_label(&sample.labels, "le");
    add_bucket(buckets, labels, bound, sample.value);
    Ok(())
}

fn add_bucket(buckets: &mut Buckets, labels: Vec<(String, String)>, bound: f64, value: f64) {
    let bounds = buckets.entry(labels).or_default();
    let mut found = false;
    for (existing, total) in bounds.iter_mut() {
        let current = *existing;
        if current.partial_cmp(&bound) == Some(Ordering::Equal) {
            *total = Add::add(*total, value);
            found = true;
        }
    }
    if !found {
        bounds.push((bound, value));
    }
}

// sorted(values.items()) with a running acc: the buckets ascending and cumulative, then the count of the last one
fn add_buckets(name: &str, samples: &mut Samples, buckets: Buckets) {
    for (labels, mut bounds) in buckets {
        bounds.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut accumulated = 0.0_f64;
        for (bound, value) in bounds {
            accumulated = Add::add(accumulated, value);
            let mut bucket_labels = labels.clone();
            bucket_labels.push(("le".to_owned(), float_to_go_string(bound)));
            let key = SampleKey {
                name: format!("{name}_bucket"),
                labels: bucket_labels,
            };
            let _previous = samples.insert(key, accumulated);
        }
        let count_key = SampleKey {
            name: format!("{name}_count"),
            labels,
        };
        let _count = samples.insert(count_key, accumulated);
    }
}
