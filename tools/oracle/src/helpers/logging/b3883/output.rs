use std::collections::BTreeMap;

use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use serde_json::{Value, json};

use super::probe::RecordedSample;

// samples keep the order the setters wrote them: remaining, max budget, remaining hours
pub(super) fn samples_output(samples: &[RecordedSample], calls: &[Value]) -> Value {
    let rows: Vec<Value> = samples.iter().map(sample_row).collect();
    json!({ "samples": rows, "lookups": calls })
}

fn sample_row(sample: &RecordedSample) -> Value {
    let labels: BTreeMap<String, String> = sample.labels.iter().cloned().collect();
    json!({
        "metric": sample.metric.clone(),
        "labels": labels,
        "value": metric_text(sample.value),
    })
}

pub(super) fn metric_text(value: MetricValue) -> String {
    match value {
        MetricValue::Finite(amount) => format!("{amount:.6}"),
        MetricValue::PositiveInfinity => "inf".to_owned(),
        MetricValue::NegativeInfinity => "-inf".to_owned(),
        MetricValue::NotANumber => "nan".to_owned(),
    }
}

pub(super) fn budget_output(row: Option<BudgetTableSnapshot>, calls: &[Value]) -> Value {
    let budget = row.map(|snapshot| {
        json!({
            "max_budget": snapshot.max_budget.map(|value| format!("{value:.6}")),
            "budget_reset_at": snapshot
                .budget_reset_at
                .map(|reset| reset.timestamp().to_string()),
        })
    });
    json!({ "budget": budget, "lookups": calls })
}
