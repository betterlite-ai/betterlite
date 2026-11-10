use std::collections::BTreeMap;

use tracing::{debug, warn};

use crate::otel::config::Signal;
use crate::otel::legacy_exporters::endpoint::normalize_otel_endpoint;
use crate::otel::legacy_exporters::headers::{HeadersError, HeadersInput, headers_dictionary};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};

const SUPPORTED_EXPORTERS: &str = "console, otlp_http, otlp_grpc";
const LOGS_EXPORTER_ENV: &str = "OTEL_LOGS_EXPORTER";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExporterKind {
    Shared,
    Console,
    OtlpHttp,
    OtlpGrpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorKind {
    Simple,
    Batch,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SpanProcessorPlan {
    pub kind: ExporterKind,
    pub endpoint: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub processor: ProcessorKind,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExporterPlan {
    pub kind: ExporterKind,
    pub endpoint: Option<String>,
    pub headers: BTreeMap<String, String>,
}

pub type LogExporterPlan = ExporterPlan;
pub type MetricReaderPlan = ExporterPlan;

// LiteLLM matches exporter text exactly, so OTLP_HTTP and GRPC fall through to the console exporter.
fn named_exporter_kind(text: &str) -> Option<ExporterKind> {
    match text {
        "console" => Some(ExporterKind::Console),
        "otlp_http" | "http/protobuf" | "http/json" => Some(ExporterKind::OtlpHttp),
        "otlp_grpc" | "grpc" => Some(ExporterKind::OtlpGrpc),
        _ => None,
    }
}

// The headers are parsed before the exporter is chosen, so OTEL_HEADERS=oops fails a console exporter too.
fn plan_headers(
    config: &OpenTelemetryConfig,
    dynamic_headers: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>, HeadersError> {
    if let Some(headers) = dynamic_headers.filter(|headers| !headers.is_empty()) {
        return headers_dictionary(HeadersInput::Map(headers));
    }
    config.headers.as_deref().map_or_else(
        || headers_dictionary(HeadersInput::None),
        |text| headers_dictionary(HeadersInput::Text(text)),
    )
}

fn otlp_endpoint(
    kind: ExporterKind,
    config: &OpenTelemetryConfig,
    signal: Signal,
    callback_name: Option<&str>,
) -> Option<String> {
    match kind {
        ExporterKind::OtlpHttp | ExporterKind::OtlpGrpc => {
            normalize_otel_endpoint(config.endpoint.as_deref(), signal, callback_name)
        }
        ExporterKind::Shared | ExporterKind::Console => None,
    }
}

pub fn span_processor_plan(
    base: &OpenTelemetryConfig,
    dynamic_headers: Option<&BTreeMap<String, String>>,
    config_override: Option<&OpenTelemetryConfig>,
    callback_name: Option<&str>,
) -> Result<SpanProcessorPlan, HeadersError> {
    let chosen = config_override.unwrap_or(base);
    let headers = plan_headers(chosen, dynamic_headers)?;
    let (kind, processor) = match &chosen.exporter {
        Exporter::InMemory => (ExporterKind::Shared, ProcessorKind::Simple),
        Exporter::Console => (ExporterKind::Console, ProcessorKind::Batch),
        Exporter::Named(text) => (
            named_exporter_kind(text).unwrap_or_else(|| {
                debug!(
                    exporter = text,
                    "otel: unknown OpenTelemetry exporter, using the console exporter"
                );
                ExporterKind::Console
            }),
            ProcessorKind::Batch,
        ),
    };
    Ok(SpanProcessorPlan {
        kind,
        endpoint: otlp_endpoint(kind, chosen, Signal::Traces, callback_name),
        headers,
        processor,
    })
}

fn log_exporter_kind(config: &OpenTelemetryConfig, env: &BTreeMap<String, String>) -> ExporterKind {
    let env_console = env
        .get(LOGS_EXPORTER_ENV)
        .is_some_and(|value| value == "console");
    match &config.exporter {
        Exporter::InMemory => ExporterKind::Shared,
        Exporter::Console => ExporterKind::Console,
        Exporter::Named(_) if env_console => ExporterKind::Console,
        Exporter::Named(text) => named_exporter_kind(text).unwrap_or_else(|| {
            warn!("OpenTelemetry: Unknown log exporter '{text}', defaulting to console. Supported: {SUPPORTED_EXPORTERS}");
            ExporterKind::Console
        }),
    }
}

// The metric reader has no shared-exporter branch, so an in-memory exporter counts as unknown here.
fn metric_exporter_kind(config: &OpenTelemetryConfig) -> ExporterKind {
    let text = match &config.exporter {
        Exporter::Console => return ExporterKind::Console,
        Exporter::Named(text) => text.clone(),
        Exporter::InMemory => "in_memory".to_owned(),
    };
    named_exporter_kind(&text).unwrap_or_else(|| {
        warn!("OpenTelemetry: Unknown metric exporter '{text}', defaulting to console. Supported: {SUPPORTED_EXPORTERS}");
        ExporterKind::Console
    })
}

fn signal_plan(
    config: &OpenTelemetryConfig,
    signal: Signal,
    kind: ExporterKind,
    callback_name: Option<&str>,
) -> Result<ExporterPlan, HeadersError> {
    let headers = plan_headers(config, None)?;
    Ok(ExporterPlan {
        kind,
        endpoint: otlp_endpoint(kind, config, signal, callback_name),
        headers,
    })
}

pub fn log_exporter_plan(
    config: &OpenTelemetryConfig,
    callback_name: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<LogExporterPlan, HeadersError> {
    signal_plan(
        config,
        Signal::Logs,
        log_exporter_kind(config, env),
        callback_name,
    )
}

pub fn metric_reader_plan(
    config: &OpenTelemetryConfig,
    callback_name: Option<&str>,
) -> Result<MetricReaderPlan, HeadersError> {
    signal_plan(
        config,
        Signal::Metrics,
        metric_exporter_kind(config),
        callback_name,
    )
}

#[path = "selection/tests.rs"]
mod tests;
