use std::collections::BTreeMap;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use serde_json::{Value, json};

use super::probe::ProbeSample;

pub(super) fn samples_value(samples: &[ProbeSample]) -> Value {
    let mut rows: Vec<(String, BTreeMap<String, String>, String)> =
        samples.iter().map(sample_row).collect();
    rows.sort();
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|(metric, labels, value)| json!({"metric": metric, "labels": labels, "value": value}))
        .collect();
    json!({ "samples": rows })
}

fn sample_row(sample: &ProbeSample) -> (String, BTreeMap<String, String>, String) {
    (
        sample.metric.clone(),
        sample.labels.iter().cloned().collect(),
        metric_text(sample.value),
    )
}

pub(super) fn metric_text(value: MetricValue) -> String {
    match value {
        MetricValue::Finite(amount) => format!("{amount:.6}"),
        MetricValue::PositiveInfinity => "inf".to_owned(),
        MetricValue::NegativeInfinity => "-inf".to_owned(),
        MetricValue::NotANumber => "nan".to_owned(),
    }
}
