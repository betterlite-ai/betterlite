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

    pub fn create_histogram(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        let metric_name = format!("litellm_{service}_{type_of_request}");
        if let Some(metric) = self.registry.get(&metric_name) {
            return Ok(metric);
        }
        let histogram = PrometheusMetric::new(
            CollectorKind::Histogram,
            &metric_name,
            &format!("Latency for {service} service"),
            &[service],
            prepare_buckets(&self.latency_buckets)?,
        );
        self.registry.register(histogram.clone())?;
        Ok(histogram)
    }

    pub fn create_gauge(
        &self,
        service: &str,
        type_of_request: &str,
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        let metric_name = format!("litellm_{service}_{type_of_request}");
        if let Some(metric) = self.registry.get(&metric_name) {
            return Ok(metric);
        }
        let gauge = PrometheusMetric::new(
            CollectorKind::Gauge,
            &metric_name,
            &format!("Gauge for {service} service"),
            &[service],
            Vec::new(),
        );
        self.registry.register(gauge.clone())?;
        Ok(gauge)
    }

    pub fn create_counter(
        &self,
        service: &str,
        type_of_request: &str,
        additional_labels: &[&str],
    ) -> Result<PrometheusMetric, PrometheusServicesError> {
        let metric_name = format!("litellm_{service}_{type_of_request}");
        if let Some(metric) = self.registry.get(&metric_name) {
            return Ok(metric);
        }
        let mut label_names = vec![service];
        label_names.extend_from_slice(additional_labels);
        let counter = PrometheusMetric::new(
            CollectorKind::Counter,
            &metric_name,
            &format!("Total {type_of_request} for {service} service"),
            &label_names,
            Vec::new(),
        );
        self.registry.register(counter.clone())?;
        Ok(counter)
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

    pub fn service_success_hook(&self, payload: &ServiceLoggerPayload) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        for metric in service_metrics {
            match metric.description.kind {
                CollectorKind::Histogram => {
                    Self::observe_histogram(metric, service, payload.duration);
                }
                CollectorKind::Counter => {
                    if metric.description.name.contains("total_requests") {
                        Self::increment_counter(metric, service, 1.0, &[]);
                    }
                }
                CollectorKind::Gauge => {}
            }
        }
    }

    pub fn service_failure_hook(&self, payload: &ServiceLoggerPayload) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        for metric in service_metrics {
            if matches!(metric.description.kind, CollectorKind::Counter) {
                Self::increment_counter(metric, service, 1.0, &[]);
            }
        }
    }

    pub fn async_service_success_hook(&self, payload: &ServiceLoggerPayload) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        for metric in service_metrics {
            match metric.description.kind {
                CollectorKind::Histogram => {
                    Self::observe_histogram(metric, service, payload.duration);
                }
                CollectorKind::Counter => {
                    if metric.description.name.contains("total_requests") {
                        Self::increment_counter(metric, service, 1.0, &[]);
                    }
                }
                CollectorKind::Gauge => {
                    if let Some(metadata) = payload.event_metadata.as_ref() {
                        let gauge_labels = metadata.gauge_labels.as_deref().unwrap_or_default();
                        let gauge_value = metadata.gauge_value.unwrap_or(0.0);
                        Self::update_gauge(metric, gauge_labels, gauge_value);
                    }
                }
            }
        }
    }

    pub fn async_service_failure_hook(&self, payload: &ServiceLoggerPayload, error_class: &str) {
        let service = wire_names::as_str(payload.service);
        let Some(service_metrics) = self.payload_to_prometheus_map.get(service) else {
            return;
        };
        for metric in service_metrics {
            match metric.description.kind {
                CollectorKind::Histogram | CollectorKind::Gauge => {}
                CollectorKind::Counter => {
                    if metric.description.name.contains("failed_requests") {
                        Self::increment_counter(
                            metric,
                            service,
                            1.0,
                            &[error_class, payload.call_type.as_str()],
                        );
                    } else {
                        Self::increment_counter(metric, service, 1.0, &[]);
                    }
                }
            }
        }
    }
}

const fn latency_bucket(bound: TimeBound) -> Bucket {
    match bound {
        TimeBound::Finite(duration) => Bucket::Seconds(duration.as_secs_f64()),
        TimeBound::Infinite => Bucket::Infinite,
    }
}
