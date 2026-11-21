use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::super::BudgetGaugeError;
use super::super::gauge::EntityBudgetGauges;
use super::super::inputs::KeyBudgetGaugeInput;
use super::super::key::set_key_budget_metrics;

// LiteLLM's refresh reads a list of key strings or full key records and sets metrics only for the full records
pub enum KeyListEntry {
    Record(KeyBudgetGaugeInput),
    Token(String),
}

pub fn set_key_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    entries: &[KeyListEntry],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for entry in entries {
        if let KeyListEntry::Record(input) = entry {
            set_key_budget_metrics(gauges, input, source, now)?;
        }
    }
    Ok(())
}
