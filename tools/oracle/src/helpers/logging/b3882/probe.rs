use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    ChildRemoval, RemovableMetric,
};
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

#[derive(Debug, thiserror::Error)]
#[error("the probe gauge refused the removal")]
pub(super) struct ProbeError;

#[derive(Clone, Debug)]
pub(super) struct ProbeSample {
    pub(super) metric: String,
    pub(super) labels: Vec<(String, String)>,
    pub(super) value: MetricValue,
}

pub(super) type ProbeLog = Arc<Mutex<Vec<ProbeSample>>>;

pub(super) fn new_log() -> ProbeLog {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn samples(log: &ProbeLog) -> Vec<ProbeSample> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[derive(Clone, Debug)]
pub(super) struct ProbeGauge {
    metric: &'static str,
    labels: Vec<(String, String)>,
    log: ProbeLog,
}

impl ProbeGauge {
    pub(super) fn new(metric: &'static str, log: &ProbeLog) -> Self {
        Self {
            metric,
            labels: Vec::new(),
            log: Arc::clone(log),
        }
    }
}

impl Collector for ProbeGauge {
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
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ProbeSample {
                    metric: self.metric.to_owned(),
                    labels: self.labels.clone(),
                    value,
                });
        }
    }
}

impl RemovableMetric for ProbeGauge {
    type Error = ProbeError;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Removed)
    }
}
