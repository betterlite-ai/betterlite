use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    ChildRemoval, RemovableMetric,
};
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

#[derive(Debug, thiserror::Error)]
#[error("the recording gauge refused the removal")]
pub(super) struct RecordError;

#[derive(Clone, Debug)]
pub(super) struct RecordedSample {
    pub(super) metric: String,
    pub(super) labels: Vec<(String, String)>,
    pub(super) value: MetricValue,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProbeLog(Arc<Mutex<Vec<RecordedSample>>>);

impl ProbeLog {
    pub(super) fn drain(&self) -> Vec<RecordedSample> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

// stands in for a registered gauge: labels() returns a child, and every set lands in the log in call order
#[derive(Clone, Debug)]
pub(super) struct RecordingGauge {
    pub(super) metric: &'static str,
    pub(super) labels: Vec<(String, String)>,
    pub(super) log: ProbeLog,
}

impl Collector for RecordingGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut child = self.clone();
        child.labels = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        child
    }

    fn update(&self, update: MetricUpdate) {
        if let MetricUpdate::Set(value) = update {
            self.log
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(RecordedSample {
                    metric: self.metric.to_owned(),
                    labels: self.labels.clone(),
                    value,
                });
        }
    }
}

impl RemovableMetric for RecordingGauge {
    type Error = RecordError;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Removed)
    }
}
