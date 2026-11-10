use std::collections::BTreeMap;

use betterlite_config::sections::otel_settings::Settings as SignalSettings;
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::trace::InMemorySpanExporter;

use crate::otel::config::ExporterSpec;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::selection::{
    ExporterKind, LogExporterPlan, MetricReaderPlan, ProcessorKind, SpanProcessorPlan,
};
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients, build_log_exporter};
use crate::otel::plumbing::processors::{ExportProcessor, processor_for};
use crate::otel::plumbing::providers::build_metric_reader;
use crate::otel::plumbing::sinks::{LogSink, MetricSink, SpanSink};

const fn exporter_kind_text(kind: ExporterKind) -> &'static str {
    match kind {
        ExporterKind::Shared => "in_memory",
        ExporterKind::Console => "console",
        ExporterKind::OtlpHttp => "otlp_http",
        ExporterKind::OtlpGrpc => "otlp_grpc",
    }
}

fn joined_headers(headers: &BTreeMap<String, String>) -> Option<String> {
    if headers.is_empty() {
        return None;
    }
    let pairs: Vec<String> = headers
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    Some(pairs.join(","))
}

// The log and metric builders read only the exporter, endpoint and headers fields of these settings.
fn signal_settings(
    kind: ExporterKind,
    endpoint: Option<String>,
    headers: &BTreeMap<String, String>,
) -> SignalSettings {
    SignalSettings {
        exporter: exporter_kind_text(kind).to_owned(),
        endpoint,
        traces_endpoint: None,
        headers: joined_headers(headers),
        service_name: String::new(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: String::new(),
        legacy_compat: false,
        langfuse_span_scope: String::new(),
        excluded_services: Vec::new(),
        exporters: Vec::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

pub struct LegacyExporterBuilder {
    factories: ExporterFactories,
    clients: HttpClients,
    shared_exporter: InMemorySpanExporter,
}

impl LegacyExporterBuilder {
    #[must_use]
    pub fn new(clients: HttpClients, shared_exporter: InMemorySpanExporter) -> Self {
        Self {
            factories: ExporterFactories::new(clients.clone(), BTreeMap::new()),
            clients,
            shared_exporter,
        }
    }

    pub fn span_processor(
        &self,
        plan: &SpanProcessorPlan,
    ) -> Result<ExportProcessor, LegacyExporterError> {
        let sink = match plan.kind {
            ExporterKind::Shared => SpanSink::InMemory(self.shared_exporter.clone()),
            ExporterKind::Console | ExporterKind::OtlpHttp | ExporterKind::OtlpGrpc => {
                self.factories.exporter_from_spec(&ExporterSpec {
                    kind: exporter_kind_text(plan.kind).to_owned(),
                    endpoint: plan.endpoint.clone(),
                    // The legacy rule already chose the exact path, which signal_endpoint would rewrite.
                    traces_endpoint: plan.endpoint.clone(),
                    headers: joined_headers(&plan.headers),
                    owner: None,
                    options: None,
                    use_simple_processor: None,
                    requires_headers: false,
                })?
            }
        };
        Ok(processor_for(
            sink,
            Some(plan.processor == ProcessorKind::Simple),
        ))
    }

    pub fn log_exporter(&self, plan: LogExporterPlan) -> Result<LogSink, LegacyExporterError> {
        Ok(build_log_exporter(
            &signal_settings(plan.kind, plan.endpoint, &plan.headers),
            &self.clients,
        )?)
    }

    pub fn metric_reader(
        &self,
        plan: MetricReaderPlan,
    ) -> Result<PeriodicReader<MetricSink>, LegacyExporterError> {
        Ok(build_metric_reader(
            &signal_settings(plan.kind, plan.endpoint, &plan.headers),
            &self.clients,
        )?)
    }
}

#[path = "build/tests.rs"]
mod tests;
