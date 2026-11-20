use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};

use crate::prometheus_helpers::bounded_series_tracker::{ChildRemoval, RemovableMetric};
use crate::prometheus_label_projection::prometheus_label_factory::ProjectedLabels;

// prometheus_client renders a label value with str(), and Python's str(None) is the text "None"; B3876 keeps the same text for counters
const MISSING_LABEL_VALUE: &str = "None";

#[derive(Clone, Debug)]
pub struct GaugeField<Gauge> {
    pub metric_name: &'static str,
    // resolved once by get_labels_for_metric (B3874), so every setter builds labels from the metric's own list
    pub supported_labels: Vec<String>,
    pub collector: Gauge,
}

// the three gauges one entity gets, in the order each setter writes them
#[derive(Clone, Debug)]
pub struct EntityBudgetGauges<Gauge> {
    pub remaining: GaugeField<Gauge>,
    pub max: GaugeField<Gauge>,
    pub remaining_hours: GaugeField<Gauge>,
}

// every budget gauge a logger registers, so the refresh job and the per-request path share one holder
#[derive(Clone, Debug)]
pub struct BudgetGauges<Gauge> {
    pub team: EntityBudgetGauges<Gauge>,
    pub org: EntityBudgetGauges<Gauge>,
    pub key: EntityBudgetGauges<Gauge>,
    pub user: EntityBudgetGauges<Gauge>,
    pub customer: EntityBudgetGauges<Gauge>,
    pub team_members: GaugeField<Gauge>,
    pub provider_remaining: GaugeField<Gauge>,
}

pub(crate) fn label_values(pairs: &[(&str, &str)]) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                LabelDumpValue::String((*value).to_owned()),
            )
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

pub(crate) fn label_pairs(labels: &ProjectedLabels) -> Vec<(&str, &str)> {
    let mut pairs = Vec::with_capacity(labels.len());
    for (name, value) in labels {
        let value = value.as_deref().unwrap_or(MISSING_LABEL_VALUE);
        pairs.push((name.as_str(), value));
    }
    pairs
}

// LiteLLM's no-op and excluded-label metrics carry no remove(), so the series tracker reads them as unsupported
impl<Gauge: RemovableMetric> RemovableMetric for MetricLike<Gauge> {
    type Error = Gauge::Error;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        match self {
            Self::NoOp(_) | Self::Excluded(_) => Ok(ChildRemoval::Unsupported),
            Self::Real(metric) => metric.remove_child(label_values),
        }
    }
}
