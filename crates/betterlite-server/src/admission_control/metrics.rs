use betterlite_prometheus::collector_registry::metric::gauge_mode::GaugeMode;
use betterlite_prometheus::collector_registry::metric::{CollectorKind, PrometheusMetric};
use betterlite_prometheus::collector_registry::{CollectorRegistry, CollectorRegistryError};

const IN_FLIGHT: &str = "litellm_in_flight_requests";
const IN_FLIGHT_HELP: &str = "Number of HTTP requests currently in-flight on this uvicorn worker";
const ADMITTED: &str = "litellm_admission_admitted_requests";
const ADMITTED_HELP: &str = "Number of requests admitted by this worker";
const QUEUED: &str = "litellm_admission_queued_requests";
const QUEUED_HELP: &str = "Number of requests queued by this worker";
const REJECTED: &str = "litellm_admission_rejected_requests_total";
const REJECTED_HELP: &str = "Number of requests rejected by this worker";
pub(super) const REASON_LABEL: &str = "reason";

// LiteLLM labels a rejection with one of the two reasons its middleware records
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RejectionReason {
    QueueFull,
    QueueTimeout,
}

impl RejectionReason {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
            Self::QueueTimeout => "queue_timeout",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct AdmissionMetrics {
    pub(super) admitted: PrometheusMetric,
    pub(super) queued: PrometheusMetric,
    pub(super) rejected: PrometheusMetric,
}

// a layer builds its metrics on its first event and never retries a refused registration
#[derive(Debug)]
pub(super) struct Registration<T> {
    attempted: bool,
    built: Option<T>,
}

impl<T: Clone> Registration<T> {
    pub(super) const fn empty() -> Self {
        Self {
            attempted: false,
            built: None,
        }
    }

    pub(super) fn once<F: FnOnce() -> Option<T>>(&mut self, build: F) -> Option<T> {
        if !self.attempted {
            self.attempted = true;
            self.built = build();
        }
        self.built.clone()
    }
}

pub(super) fn in_flight_gauge(registry: &CollectorRegistry) -> Option<PrometheusMetric> {
    register(registry, live_sum_gauge(IN_FLIGHT, IN_FLIGHT_HELP))
        .inspect_err(|error| {
            tracing::warn!(%error, "could not register the in-flight requests gauge");
        })
        .ok()
}

// create_prometheus_admission_metrics: a refused metric leaves the layer without the three
pub(super) fn admission_metrics(registry: &CollectorRegistry) -> Option<AdmissionMetrics> {
    match register_admission_metrics(registry) {
        Ok(metrics) => Some(metrics),
        Err(error) => {
            tracing::warn!(%error, "could not register the admission control metrics");
            None
        }
    }
}

fn register_admission_metrics(
    registry: &CollectorRegistry,
) -> Result<AdmissionMetrics, CollectorRegistryError> {
    let admitted = register(registry, live_sum_gauge(ADMITTED, ADMITTED_HELP))?;
    let queued = register(registry, live_sum_gauge(QUEUED, QUEUED_HELP))?;
    let rejected = register(
        registry,
        PrometheusMetric::new(
            CollectorKind::Counter,
            REJECTED,
            REJECTED_HELP,
            &[REASON_LABEL],
            Vec::new(),
        ),
    )?;
    Ok(AdmissionMetrics {
        admitted,
        queued,
        rejected,
    })
}

fn register(
    registry: &CollectorRegistry,
    metric: PrometheusMetric,
) -> Result<PrometheusMetric, CollectorRegistryError> {
    registry.register(metric.clone())?;
    Ok(metric)
}

// both of LiteLLM's branches land here: only B10096's writer reads the mode, and it exists under PROMETHEUS_MULTIPROC_DIR
fn live_sum_gauge(name: &str, help: &str) -> PrometheusMetric {
    PrometheusMetric::new(CollectorKind::Gauge, name, help, &[], Vec::new())
        .with_multiprocess_mode(GaugeMode::LiveSum)
}
