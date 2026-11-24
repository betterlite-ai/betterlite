use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Neg as _;
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::collector_factory::collector::MetricValue;
use crate::collector_registry::metric::sample::Sample;
use crate::collector_registry::metric::{
    Bucket, CollectorDescription, CollectorKind, PrometheusMetric,
};

use super::float::float_to_go_string;
use super::{MetricFamily, MetricKind, MetricSample};

pub(super) mod openmetrics;

// one family's samples after the name[] filter: every sample, or only the ones whose full name was asked for
pub(super) struct Restricted<'families> {
    pub(super) family: &'families MetricFamily,
    pub(super) samples: Vec<&'families MetricSample>,
}

// RestrictedRegistry.collect followed by _restricted_metric: a family left without samples is dropped
pub(super) fn restricted<'families>(
    families: &'families [MetricFamily],
    metric_names: &[String],
) -> Vec<Restricted<'families>> {
    let names: Option<HashSet<&str>> = if metric_names.is_empty() {
        None
    } else {
        Some(metric_names.iter().map(String::as_str).collect())
    };
    let mut selected = Vec::new();
    for family in families {
        let samples = match names.as_ref() {
            None => family.samples.iter().collect(),
            Some(requested) => {
                let kept: Vec<&MetricSample> = family
                    .samples
                    .iter()
                    .filter(|sample| requested.contains(sample.name.as_str()))
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                kept
            }
        };
        selected.push(Restricted { family, samples });
    }
    selected
}

// generate_latest: munged counter names, the family's samples and, in the text format, a trailing _created gauge block
pub(super) fn render_text(families: &[MetricFamily], metric_names: &[String]) -> Vec<u8> {
    let mut output = String::new();
    for selected in restricted(families, metric_names) {
        let family = selected.family;
        let recorded_name = match family.kind {
            MetricKind::Counter => format!("{}_total", family.name),
            MetricKind::Gauge | MetricKind::Summary | MetricKind::Histogram => family.name.clone(),
        };
        let help = escape_help(&family.help, false);
        let _help_line = writeln!(output, "# HELP {recorded_name} {help}");
        let _type_line = writeln!(output, "# TYPE {recorded_name} {}", family.kind.as_str());
        let created_name = format!("{}_created", family.name);
        for sample in selected
            .samples
            .iter()
            .filter(|sample| sample.name != created_name)
        {
            output.push_str(&sample_line(sample));
        }
        let created: Vec<&MetricSample> = selected
            .samples
            .iter()
            .copied()
            .filter(|sample| sample.name == created_name)
            .collect();
        if !created.is_empty() {
            let _created_help_line = writeln!(output, "# HELP {created_name} {help}");
            let _created_type_line = writeln!(output, "# TYPE {created_name} gauge");
            for sample in created {
                output.push_str(&sample_line(sample));
            }
        }
    }
    output.into_bytes()
}

// the registry's metrics as the families the encoders write, in the order the registry holds them
pub(super) fn families_of(metrics: &[PrometheusMetric]) -> Vec<MetricFamily> {
    let mut families = Vec::with_capacity(metrics.len());
    for metric in metrics {
        families.push(family(metric));
    }
    families
}

fn family(metric: &PrometheusMetric) -> MetricFamily {
    let description = &metric.description;
    let kind = match description.kind {
        CollectorKind::Counter => MetricKind::Counter,
        CollectorKind::Gauge => MetricKind::Gauge,
        CollectorKind::Histogram => MetricKind::Histogram,
    };
    MetricFamily {
        name: description.name.clone(),
        help: description.help.clone(),
        kind,
        samples: samples_of(metric),
    }
}

fn samples_of(metric: &PrometheusMetric) -> Vec<MetricSample> {
    let description = &metric.description;
    let mut samples = Vec::new();
    for (values, sample) in metric.series_in_use_order() {
        let labels: Vec<(String, String)> = description
            .label_names
            .iter()
            .cloned()
            .zip(values)
            .collect();
        match description.kind {
            CollectorKind::Counter => samples.extend(counter_samples(description, &sample, labels)),
            CollectorKind::Gauge => samples.push(MetricSample {
                name: description.name.clone(),
                labels,
                value: metric_value(sample.total),
            }),
            CollectorKind::Histogram => {
                samples.extend(histogram_samples(description, &sample, labels));
            }
        }
    }
    samples
}

// one sample of a family: the suffix tells its exposed name from the registered one
fn metric_sample(
    description: &CollectorDescription,
    suffix: &str,
    labels: Vec<(String, String)>,
    value: f64,
) -> MetricSample {
    MetricSample {
        name: format!("{}{suffix}", description.name),
        labels,
        value,
    }
}

fn counter_samples(
    description: &CollectorDescription,
    sample: &Sample,
    labels: Vec<(String, String)>,
) -> Vec<MetricSample> {
    vec![
        metric_sample(
            description,
            "_total",
            labels.clone(),
            metric_value(sample.total),
        ),
        metric_sample(
            description,
            "_created",
            labels,
            created_seconds(sample.created),
        ),
    ]
}

// one bucket line: its upper bound as le and the count of every observation it covers
fn bucket_sample(
    description: &CollectorDescription,
    bound: &Bucket,
    labels: &[(String, String)],
    count: u64,
) -> MetricSample {
    let mut bucket_labels = labels.to_vec();
    bucket_labels.push(("le".to_owned(), float_to_go_string(bound.key())));
    metric_sample(
        description,
        "_bucket",
        bucket_labels,
        Decimal::from(count).to_f64().unwrap_or(f64::NAN),
    )
}

// Histogram._child_samples: the cumulative buckets, the count of the last one, the sum for a first bound of zero or more, then the creation time
fn histogram_samples(
    description: &CollectorDescription,
    sample: &Sample,
    labels: Vec<(String, String)>,
) -> Vec<MetricSample> {
    let mut samples = Vec::new();
    for (index, bound) in description.buckets.iter().enumerate() {
        let count = sample.buckets.get(index).copied().unwrap_or_default();
        samples.push(bucket_sample(description, bound, &labels, count));
    }
    let last_count = sample.buckets.last().copied().unwrap_or(sample.count);
    samples.push(metric_sample(
        description,
        "_count",
        labels.clone(),
        Decimal::from(last_count).to_f64().unwrap_or(f64::NAN),
    ));
    if description
        .buckets
        .first()
        .is_none_or(|bound| bound.key() >= 0.0_f64)
    {
        samples.push(metric_sample(
            description,
            "_sum",
            labels.clone(),
            metric_value(sample.total),
        ));
    }
    samples.push(metric_sample(
        description,
        "_created",
        labels,
        created_seconds(sample.created),
    ));
    samples
}

fn metric_value(value: MetricValue) -> f64 {
    match value {
        MetricValue::Finite(number) => number.to_f64().unwrap_or(f64::NAN),
        MetricValue::PositiveInfinity => f64::INFINITY,
        MetricValue::NegativeInfinity => f64::NEG_INFINITY,
        MetricValue::NotANumber => f64::NAN,
    }
}

fn created_seconds(created: SystemTime) -> f64 {
    match created.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(error) => error.duration().as_secs_f64().neg(),
    }
}

pub(super) fn sample_line(sample: &MetricSample) -> String {
    let mut sorted = sample.labels.clone();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let pairs: Vec<String> = sorted
        .iter()
        .map(|(name, value)| format!("{name}=\"{}\"", escaped_label_value(value)))
        .collect();
    let labels = if pairs.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", pairs.join(","))
    };
    format!(
        "{}{labels} {}\n",
        sample.name,
        float_to_go_string(sample.value)
    )
}

pub(super) fn escape_help(help: &str, escape_quote: bool) -> String {
    let escaped = help.replace('\\', "\\\\").replace('\n', "\\n");
    if escape_quote {
        escaped.replace('"', "\\\"")
    } else {
        escaped
    }
}

fn escaped_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}
