use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    ChildRemoval, RemovableMetric,
};
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

use super::{Outcome, TrackScenario};

#[derive(Debug, thiserror::Error)]
#[error("the probe metric refused the removal")]
pub(super) struct ProbeError;

#[derive(Debug)]
pub(super) enum ProbeEvent {
    Increment {
        metric: String,
        labels: BTreeMap<String, String>,
        amount: String,
    },
    Removal {
        metric: String,
        labels: Vec<Option<String>>,
    },
}

#[derive(Debug, Default)]
pub(super) struct ProbeLog {
    pub(super) events: Vec<ProbeEvent>,
}

#[derive(Clone, Debug)]
pub(super) struct ProbeMetric {
    pub(super) name: String,
    pub(super) outcome: Outcome,
    pub(super) labels: Vec<(String, String)>,
    pub(super) log: Arc<Mutex<ProbeLog>>,
}

impl Collector for ProbeMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut child = self.clone();
        child.labels = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        child
    }

    fn update(&self, update: MetricUpdate) {
        if let MetricUpdate::Inc(value) = update {
            let labels = self.labels.iter().cloned().collect();
            let amount = amount_text(value);
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .events
                .push(ProbeEvent::Increment {
                    metric: self.name.clone(),
                    labels,
                    amount,
                });
        }
    }
}

impl RemovableMetric for ProbeMetric {
    type Error = ProbeError;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .push(ProbeEvent::Removal {
                metric: self.name.clone(),
                labels: label_values.to_vec(),
            });
        match self.outcome {
            Outcome::Removed => Ok(ChildRemoval::Removed),
            Outcome::Absent => Ok(ChildRemoval::Absent),
            Outcome::Unsupported => Ok(ChildRemoval::Unsupported),
            Outcome::Failed => Err(ProbeError),
        }
    }
}

pub(super) fn probe_metrics(
    scenario: &TrackScenario,
    log: &Arc<Mutex<ProbeLog>>,
) -> BTreeMap<String, ProbeMetric> {
    scenario
        .metrics
        .iter()
        .map(|(name, outcome)| {
            let metric = ProbeMetric {
                name: name.clone(),
                outcome: *outcome,
                labels: Vec::new(),
                log: Arc::clone(log),
            };
            (name.clone(), metric)
        })
        .collect()
}

fn amount_text(value: MetricValue) -> String {
    match value {
        MetricValue::Finite(decimal) => decimal.to_string(),
        MetricValue::PositiveInfinity => "Infinity".to_owned(),
        MetricValue::NegativeInfinity => "-Infinity".to_owned(),
        MetricValue::NotANumber => "NaN".to_owned(),
    }
}
