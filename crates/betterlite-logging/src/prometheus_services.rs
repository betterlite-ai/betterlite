use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_prometheus::buckets::{LATENCY_BUCKETS, TimeBound};
use betterlite_prometheus::collector_factory::collector::Collector as _;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::{
    Bucket, CollectorKind, PrometheusMetric, prepare_buckets,
};

use crate::logging_service_events::types::ServiceLoggerPayload;
use crate::logging_service_events::types::ServiceMetric;
use crate::logging_service_events::types::ServiceType;
use crate::logging_service_events::types::wire_names;

use self::error::PrometheusServicesError;
use self::metrics::label_pairs;
use self::metrics::metric_value;

pub mod error;

mod metrics;
mod tests;

const FAILED_REQUESTS_LABELS: [&str; 2] = ["error_class", "function_name"];

#[derive(Clone, Copy)]
enum ServiceEvent<'event> {
    Success,
    Failure,
    FailureWithError(&'event str),
}

pub trait ServiceHealthMetrics {
    fn create_histogram(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError>;

    fn create_gauge(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError>;

    fn create_counter(
        &self,
        service: &str,
        type_of_request: &str,
        additional_labels: &[&str],
    ) -> Result<PrometheusMetric, PrometheusServicesError>;

    fn service_success_hook(&self, payload: &ServiceLoggerPayload);

    fn service_failure_hook(&self, payload: &ServiceLoggerPayload);

    fn async_service_success_hook(&self, payload: &ServiceLoggerPayload);

    fn async_service_failure_hook(&self, payload: &ServiceLoggerPayload, error_class: &str);
}

#[derive(Debug)]
pub struct PrometheusServicesLogger {
    registry: Arc<CollectorRegistry>,
    latency_buckets: Vec<Bucket>,
    payload_to_prometheus_map: BTreeMap<String, Vec<PrometheusMetric>>,
}

impl PrometheusServicesLogger {
    pub fn new(
        registry: Arc<CollectorRegistry>,
        latency_buckets: Option<&[f64]>,
    ) -> Result<Self, PrometheusServicesError> {
        Self::construct(registry, latency_buckets).inspect_err(|error| {
            tracing::debug!("Got exception on init prometheus client {error}");
        })
    }

    fn construct(
        registry: Arc<CollectorRegistry>,
        latency_buckets: Option<&[f64]>,
    ) -> Result<Self, PrometheusServicesError> {
        let bounds: Vec<Bucket> = latency_buckets.map_or_else(
            || {
                LATENCY_BUCKETS
                    .iter()
                    .copied()
                    .map(latency_bucket)
                    .collect()
            },
            |seconds| seconds.iter().copied().map(Bucket::Seconds).collect(),
        );
        let mut logger = Self {
            registry,
            latency_buckets: bounds,
            payload_to_prometheus_map: BTreeMap::new(),
        };
        for service in ServiceType::ALL {
            logger.initialize_service(service)?;
        }
        Ok(logger)
    }

    fn initialize_service(&mut self, service: ServiceType) -> Result<(), PrometheusServicesError> {
        let service_name = wire_names::as_str(service);
        let configured = service.metrics();
        let mut metrics = Vec::new();
        if configured.contains(&ServiceMetric::Histogram) {
            metrics.push(self.create_histogram(service_name, "latency")?);
        }
        if configured.contains(&ServiceMetric::Counter) {
            metrics.push(self.create_counter(
                service_name,
                "failed_requests",
                &FAILED_REQUESTS_LABELS,
            )?);
            metrics.push(self.create_counter(service_name, "total_requests", &[])?);
        }
        if configured.contains(&ServiceMetric::Gauge) {
            metrics.push(self.create_gauge(service_name, "size")?);
        }
        if !metrics.is_empty() {
            self.payload_to_prometheus_map
                .insert(service_name.to_owned(), metrics);
        }
        Ok(())
    }

    fn create_metric(
        &self,
        kind: CollectorKind,
        service: &str,
        type_of_request: &str,
        additional_labels: &[&str],
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        let metric_name = format!("litellm_{service}_{type_of_request}");
        if let Some(existing) = self.registry.get(&metric_name) {
            return Ok(existing);
        }
        let mut label_names = vec![service];
        label_names.extend_from_slice(additional_labels);
        let help = match kind {
            CollectorKind::Histogram => format!("Latency for {service} service"),
            CollectorKind::Gauge => format!("Gauge for {service} service"),
            CollectorKind::Counter => format!("Total {type_of_request} for {service} service"),
        };
        let buckets = match kind {
            CollectorKind::Histogram => prepare_buckets(&self.latency_buckets)?,
            CollectorKind::Gauge | CollectorKind::Counter => Vec::new(),
        };
        let metric = PrometheusMetric::new(kind, &metric_name, &help, &label_names, buckets);
        self.registry.register(metric.clone())?;
        Ok(metric)
    }

    pub fn observe_histogram(histogram: &PrometheusMetric, labels: &str, amount: f64) {
        histogram
            .labels(&label_pairs(&histogram.description.label_names, &[labels]))
            .observe(metric_value(amount));
    }

    pub fn update_gauge(gauge: &PrometheusMetric, labels: &str, amount: f64) {
        gauge
            .labels(&label_pairs(&gauge.description.label_names, &[labels]))
            .set(metric_value(amount));
    }

    pub fn increment_counter(
        counter: &PrometheusMetric,
        labels: &str,
        amount: f64,
        additional_labels: &[&str],
    ) {
        let mut values = vec![labels];
        values.extend_from_slice(additional_labels);
        counter
            .labels(&label_pairs(&counter.description.label_names, &values))
            .inc(metric_value(amount));
    }

    fn log_event(&self, payload: &ServiceLoggerPayload, event: ServiceEvent<'_>) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        for metric in service_metrics {
            Self::log_metric(payload, metric, service, event);
        }
    }

    fn log_metric(
        payload: &ServiceLoggerPayload,
        metric: &PrometheusMetric,
        service: &str,
        event: ServiceEvent<'_>,
    ) {
        match (event, metric.description.kind) {
            (ServiceEvent::Success, CollectorKind::Histogram) => {
                Self::observe_histogram(metric, service, payload.duration);
            }
            (ServiceEvent::Success, CollectorKind::Counter) => {
                if metric.description.name.contains("total_requests") {
                    Self::increment_counter(metric, service, 1.0, &[]);
                }
            }
            (ServiceEvent::FailureWithError(error_class), CollectorKind::Counter) => {
                let extra_labels = [error_class, payload.call_type.as_str()];
                let additional: &[&str] = if metric.description.name.contains("failed_requests") {
                    &extra_labels
                } else {
                    &[]
                };
                Self::increment_counter(metric, service, 1.0, additional);
            }
            (ServiceEvent::Failure, CollectorKind::Counter) => {
                Self::increment_counter(metric, service, 1.0, &[]);
            }
            (ServiceEvent::Success, CollectorKind::Gauge)
            | (
                ServiceEvent::Failure | ServiceEvent::FailureWithError(_),
                CollectorKind::Histogram | CollectorKind::Gauge,
            ) => {}
        }
    }

    fn update_service_gauges(&self, payload: &ServiceLoggerPayload) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        let Some(metadata) = payload.event_metadata.as_ref() else {
            return;
        };
        let labels = metadata.gauge_labels.as_deref().unwrap_or_default();
        for metric in service_metrics {
            if matches!(metric.description.kind, CollectorKind::Gauge) {
                Self::update_gauge(metric, labels, metadata.gauge_value.unwrap_or(0.0));
            }
        }
    }
}

impl ServiceHealthMetrics for PrometheusServicesLogger {
    fn create_histogram(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        self.create_metric(CollectorKind::Histogram, service, type_of_request, &[])
    }

    fn create_gauge(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        self.create_metric(CollectorKind::Gauge, service, type_of_request, &[])
    }

    fn create_counter(
        &self,
        service: &str,
        type_of_request: &str,
        additional_labels: &[&str],
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        self.create_metric(
            CollectorKind::Counter,
            service,
            type_of_request,
            additional_labels,
        )
    }

    fn service_success_hook(&self, payload: &ServiceLoggerPayload) {
        self.log_event(payload, ServiceEvent::Success);
    }

    fn service_failure_hook(&self, payload: &ServiceLoggerPayload) {
        self.log_event(payload, ServiceEvent::Failure);
    }

    fn async_service_success_hook(&self, payload: &ServiceLoggerPayload) {
        self.service_success_hook(payload);
        self.update_service_gauges(payload);
    }

    fn async_service_failure_hook(&self, payload: &ServiceLoggerPayload, error_class: &str) {
        self.log_event(payload, ServiceEvent::FailureWithError(error_class));
    }
}

const fn latency_bucket(bound: TimeBound) -> Bucket {
    match bound {
        TimeBound::Finite(duration) => Bucket::Seconds(duration.as_secs_f64()),
        TimeBound::Infinite => Bucket::Infinite,
    }
}
