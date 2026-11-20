use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use betterlite_logging::prometheus_usage::counters::UsageCounters;
use betterlite_logging::prometheus_usage::metric::UsageMetric;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

#[derive(Debug, thiserror::Error)]
#[error("the probe metric refused the removal")]
pub(super) struct ProbeError;

#[derive(Debug)]
pub(super) enum ProbeEvent {
    Update {
        event: &'static str,
        metric: String,
        labels: BTreeMap<String, String>,
        amount: String,
    },
}

#[derive(Debug, Default)]
pub(super) struct ProbeLog {
    pub(super) events: Vec<ProbeEvent>,
}

#[derive(Clone, Debug)]
pub(super) struct ProbeMetric {
    pub(super) name: String,
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
        let (event, amount) = match update {
            MetricUpdate::Inc(value) => ("increment", amount_text(value)),
            MetricUpdate::Set(value) => ("set", amount_text(value)),
            MetricUpdate::Observe(value) => ("observe", amount_text(value)),
            MetricUpdate::Remove => ("remove", String::new()),
        };
        let labels = self.labels.iter().cloned().collect();
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .push(ProbeEvent::Update {
                event,
                metric: self.name.clone(),
                labels,
                amount,
            });
    }
}

impl RemovableMetric for ProbeMetric {
    type Error = ProbeError;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Removed)
    }
}

pub(super) struct ProbeCounters {
    supported: BTreeMap<String, Vec<String>>,
    tracker: BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
    pub(super) log: Arc<Mutex<ProbeLog>>,
}

impl ProbeCounters {
    pub(super) fn new(supported: &BTreeMap<String, Vec<String>>) -> Self {
        Self {
            supported: supported.clone(),
            tracker: BoundedPrometheusSeriesTracker::default(),
            limits: SeriesLimits::default(),
            log: Arc::new(Mutex::new(ProbeLog::default())),
        }
    }
}

impl UsageCounters for ProbeCounters {
    type Counter = ProbeMetric;

    fn counter(&self, metric: UsageMetric) -> Self::Counter {
        ProbeMetric {
            name: metric.name().to_owned(),
            labels: Vec::new(),
            log: Arc::clone(&self.log),
        }
    }

    fn supported_labels(&self, metric: UsageMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.tracker
    }

    fn limits(&self) -> &SeriesLimits {
        &self.limits
    }
}

fn amount_text(value: MetricValue) -> String {
    match value {
        MetricValue::Finite(decimal) => decimal.normalize().to_string(),
        MetricValue::PositiveInfinity => "Infinity".to_owned(),
        MetricValue::NegativeInfinity => "-Infinity".to_owned(),
        MetricValue::NotANumber => "NaN".to_owned(),
    }
}
