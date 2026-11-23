use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Neg as _;
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::collector_factory::collector::MetricValue;
use crate::collector_registry::CollectorRegistry;
use crate::collector_registry::metric::sample::Sample;
use crate::collector_registry::metric::{CollectorDescription, CollectorKind, PrometheusMetric};

use super::float::float_to_go_string;

pub(super) mod openmetrics;

// one registered family's samples, already in prometheus_client's order and formatted for the wire
pub(super) struct Family {
    pub(super) name: String,
    pub(super) kind: CollectorKind,
    pub(super) help: String,
    pub(super) samples: Vec<FamilySample>,
}

pub(super) struct FamilySample {
    pub(super) name: String,
    pub(super) labels: Vec<(String, String)>,
    pub(super) value: String,
    pub(super) created: bool,
}

// generate_latest: munged counter names, the family's samples and, in the text format, a trailing _created gauge block
pub(super) fn render_text(registry: &CollectorRegistry, metric_names: &[String]) -> Vec<u8> {
    let mut output = String::new();
    for family in families(registry, metric_names) {
        let recorded_name = match family.kind {
            CollectorKind::Counter => format!("{}_total", family.name),
            CollectorKind::Gauge | CollectorKind::Histogram => family.name.clone(),
        };
        let help = escape_help(&family.help, false);
        let _help_line = writeln!(output, "# HELP {recorded_name} {help}");
        let _type_line = writeln!(output, "# TYPE {recorded_name} {}", family.kind.as_str());
        for sample in family.samples.iter().filter(|sample| !sample.created) {
            output.push_str(&sample_line(sample));
        }
        let created: Vec<&FamilySample> = family
            .samples
            .iter()
            .filter(|sample| sample.created)
            .collect();
        if !created.is_empty() {
            let created_help = escape_help(&family.help, false);
            let _created_help_line =
                writeln!(output, "# HELP {}_created {created_help}", family.name);
            let _created_type_line = writeln!(output, "# TYPE {}_created gauge", family.name);
            for sample in created {
                output.push_str(&sample_line(sample));
            }
        }
    }
    output.into_bytes()
}

// RestrictedRegistry.collect followed by collect(): participating families only, then their samples whose full name is asked for
pub(super) fn families(registry: &CollectorRegistry, metric_names: &[String]) -> Vec<Family> {
    let names: Option<HashSet<&str>> = if metric_names.is_empty() {
        None
    } else {
        Some(metric_names.iter().map(String::as_str).collect())
    };
    registry
        .metrics()
        .iter()
        .filter_map(|metric| family(metric, names.as_ref()))
        .collect()
}

fn family(metric: &PrometheusMetric, names: Option<&HashSet<&str>>) -> Option<Family> {
    let mut samples = samples_of(metric);
    if let Some(names) = names {
        let participates = metric
            .sample_names()
            .iter()
            .any(|name| names.contains(name.as_str()));
        if !participates {
            return None;
        }
        samples.retain(|sample| names.contains(sample.name.as_str()));
        if samples.is_empty() {
            return None;
        }
    }
    let description = &metric.description;
    Some(Family {
        name: description.name.clone(),
        kind: description.kind,
        help: description.help.clone(),
        samples,
    })
}

fn samples_of(metric: &PrometheusMetric) -> Vec<FamilySample> {
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
            CollectorKind::Counter => {
                samples.extend(counter_samples(description, &sample, labels));
            }
            CollectorKind::Gauge => samples.push(FamilySample {
                name: description.name.clone(),
                labels,
                value: metric_value_text(sample.total),
                created: false,
            }),
            CollectorKind::Histogram => {
                samples.extend(histogram_samples(description, &sample, labels));
            }
        }
    }
    samples
}

fn counter_samples(
    description: &CollectorDescription,
    sample: &Sample,
    labels: Vec<(String, String)>,
) -> Vec<FamilySample> {
    vec![
        FamilySample {
            name: format!("{}_total", description.name),
            labels: labels.clone(),
            value: metric_value_text(sample.total),
            created: false,
        },
        FamilySample {
            name: format!("{}_created", description.name),
            labels,
            value: created_text(sample.created),
            created: true,
        },
    ]
}

// Histogram._child_samples: the cumulative buckets, the count of the last one, the sum for a first bound of zero or more, then the creation time
fn histogram_samples(
    description: &CollectorDescription,
    sample: &Sample,
    labels: Vec<(String, String)>,
) -> Vec<FamilySample> {
    let mut samples = Vec::new();
    for (index, bound) in description.buckets.iter().enumerate() {
        let count = sample.buckets.get(index).copied().unwrap_or_default();
        let mut bucket_labels = labels.clone();
        bucket_labels.push(("le".to_owned(), float_to_go_string(bound.key())));
        samples.push(FamilySample {
            name: format!("{}_bucket", description.name),
            labels: bucket_labels,
            value: float_to_go_string(Decimal::from(count).to_f64().unwrap_or(f64::NAN)),
            created: false,
        });
    }
    samples.push(FamilySample {
        name: format!("{}_count", description.name),
        labels: labels.clone(),
        value: float_to_go_string(
            Decimal::from(sample.buckets.last().copied().unwrap_or(sample.count))
                .to_f64()
                .unwrap_or(f64::NAN),
        ),
        created: false,
    });
    if description
        .buckets
        .first()
        .is_none_or(|bound| bound.key() >= 0.0)
    {
        samples.push(FamilySample {
            name: format!("{}_sum", description.name),
            labels: labels.clone(),
            value: metric_value_text(sample.total),
            created: false,
        });
    }
    samples.push(FamilySample {
        name: format!("{}_created", description.name),
        labels,
        value: created_text(sample.created),
        created: true,
    });
    samples
}

fn metric_value_text(value: MetricValue) -> String {
    let number = match value {
        MetricValue::Finite(number) => number.to_f64().unwrap_or(f64::NAN),
        MetricValue::PositiveInfinity => f64::INFINITY,
        MetricValue::NegativeInfinity => f64::NEG_INFINITY,
        MetricValue::NotANumber => f64::NAN,
    };
    float_to_go_string(number)
}

fn created_text(created: SystemTime) -> String {
    let seconds = match created.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(error) => error.duration().as_secs_f64().neg(),
    };
    float_to_go_string(seconds)
}

pub(super) fn sample_line(sample: &FamilySample) -> String {
    let labels = label_text(&sample.labels);
    format!("{}{labels} {}\n", sample.name, sample.value)
}

pub(super) fn escape_help(help: &str, escape_quote: bool) -> String {
    let escaped = help.replace('\\', "\\\\").replace('\n', "\\n");
    if escape_quote {
        escaped.replace('"', "\\\"")
    } else {
        escaped
    }
}

fn label_text(labels: &[(String, String)]) -> String {
    let mut sorted = labels.to_vec();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let pairs: Vec<String> = sorted
        .iter()
        .map(|(name, value)| format!("{name}=\"{}\"", escaped_label_value(value)))
        .collect();
    if pairs.is_empty() {
        return String::new();
    }
    format!("{{{}}}", pairs.join(","))
}

fn escaped_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}
