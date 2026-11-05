use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use opentelemetry_http::HttpClient;
use opentelemetry_otlp::tonic_types::metadata::MetadataMap;
use opentelemetry_otlp::{
    ExporterBuildError, Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig,
};
use opentelemetry_sdk::logs::InMemoryLogExporter;
use opentelemetry_sdk::trace::InMemorySpanExporter;

use crate::otel::config::{ExporterSpec, Signal};
use crate::otel::plumbing::endpoints::signal_endpoint;
use crate::otel::plumbing::headers::parse_headers;
use crate::otel::plumbing::sinks::{LogSink, MetricSink, SpanSink};

pub type ExporterFactory =
    Arc<dyn Fn(&ExporterSpec) -> Result<SpanSink, ExporterError> + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum ExporterError {
    #[error("unrecognized exporter kind {0:?}")]
    UnknownKind(String),
    #[error("invalid OTLP header name: {0}")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("invalid OTLP header value: {0}")]
    HeaderValue(#[from] InvalidHeaderValue),
    #[error("building the OTLP exporter failed: {0}")]
    Build(#[from] ExporterBuildError),
}

// One client per signal, because the TLS policy is resolved per signal.
#[derive(Clone)]
pub struct HttpClients {
    pub traces: Arc<dyn HttpClient>,
    pub metrics: Arc<dyn HttpClient>,
    pub logs: Arc<dyn HttpClient>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Http,
    Grpc,
    Headerless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Console,
    InMemory,
    HttpJson,
    HttpProto,
    Grpc,
    Unrecognized,
}

fn classify(kind: &str) -> Kind {
    match kind.to_lowercase().as_str() {
        "" | "console" => Kind::Console,
        "in_memory" | "inmemory" | "memory" => Kind::InMemory,
        "http/json" => Kind::HttpJson,
        "otlp_http" | "http" | "http/protobuf" => Kind::HttpProto,
        "otlp_grpc" | "grpc" => Kind::Grpc,
        _ => Kind::Unrecognized,
    }
}

// The SDK percent-decodes header values it is given, and parse_headers already did.
fn http_exporter<B>(
    builder: B,
    protocol: Protocol,
    endpoint: Option<String>,
    raw_headers: Option<&str>,
    client: &Arc<dyn HttpClient>,
) -> B
where
    B: WithExportConfig + WithHttpConfig,
{
    let headers: HashMap<String, String> = parse_headers(raw_headers)
        .into_iter()
        .map(|(name, value)| (name, value.replace('%', "%25")))
        .collect();
    let configured = builder
        .with_protocol(protocol)
        .with_shared_http_client(Arc::clone(client))
        .with_headers(headers);
    match endpoint.filter(|url| !url.is_empty()) {
        Some(url) => configured.with_endpoint(url),
        None => configured,
    }
}

fn grpc_exporter<B>(
    builder: B,
    endpoint: Option<&str>,
    raw_headers: Option<&str>,
) -> Result<B, ExporterError>
where
    B: WithExportConfig + WithTonicConfig,
{
    let mut headers = HeaderMap::new();
    for (name, value) in parse_headers(raw_headers) {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(&value)?,
        );
    }
    let configured = builder.with_metadata(MetadataMap::from_headers(headers));
    Ok(match endpoint.filter(|url| !url.is_empty()) {
        Some(url) => configured.with_endpoint(url),
        None => configured,
    })
}

pub struct ExporterFactories {
    factories: BTreeMap<String, ExporterFactory>,
    clients: HttpClients,
}

impl ExporterFactories {
    // Custom kinds come in as an explicit list, keyed by the kind the operator spells in config.
    #[must_use]
    pub fn new(clients: HttpClients, custom: BTreeMap<String, ExporterFactory>) -> Self {
        Self {
            factories: custom
                .into_iter()
                .map(|(kind, factory)| (kind.to_lowercase(), factory))
                .collect(),
            clients,
        }
    }

    // Routing reads this rather than a denylist, so a mistyped kind is never taken for OTLP.
    #[must_use]
    pub fn exporter_transport(&self, kind: &str) -> Transport {
        match classify(kind) {
            Kind::HttpJson | Kind::HttpProto => Transport::Http,
            Kind::Grpc => Transport::Grpc,
            Kind::Console | Kind::InMemory | Kind::Unrecognized => {
                if self.factories.contains_key(&kind.to_lowercase()) {
                    Transport::Http
                } else {
                    Transport::Headerless
                }
            }
        }
    }

    pub fn exporter_from_spec(&self, spec: &ExporterSpec) -> Result<SpanSink, ExporterError> {
        let lowered = spec.kind.to_lowercase();
        if let Some(factory) = self.factories.get(&lowered) {
            return factory(spec);
        }
        let headers = spec.headers.as_deref();
        match classify(&lowered) {
            Kind::Console => Ok(SpanSink::Console(
                opentelemetry_stdout::SpanExporter::default(),
            )),
            Kind::InMemory => Ok(SpanSink::InMemory(InMemorySpanExporter::default())),
            kind @ (Kind::HttpJson | Kind::HttpProto) => {
                let protocol = if kind == Kind::HttpJson {
                    Protocol::HttpJson
                } else {
                    Protocol::HttpBinary
                };
                let endpoint = spec
                    .traces_endpoint
                    .clone()
                    .filter(|url| !url.is_empty())
                    .or_else(|| signal_endpoint(spec.endpoint.as_deref(), Signal::Traces));
                let builder = opentelemetry_otlp::SpanExporter::builder().with_http();
                let configured =
                    http_exporter(builder, protocol, endpoint, headers, &self.clients.traces);
                Ok(SpanSink::Otlp(Box::new(configured.build()?)))
            }
            Kind::Grpc => {
                let builder = opentelemetry_otlp::SpanExporter::builder().with_tonic();
                let configured = grpc_exporter(builder, spec.endpoint.as_deref(), headers)?;
                Ok(SpanSink::Otlp(Box::new(configured.build()?)))
            }
            Kind::Unrecognized => Err(ExporterError::UnknownKind(spec.kind.clone())),
        }
    }

    // The top-level exporter/endpoint/headers fields are the one-exporter shorthand.
    pub fn build_span_exporter(&self, config: &OtelConfig) -> Result<SpanSink, ExporterError> {
        self.exporter_from_spec(&ExporterSpec {
            kind: config.exporter.clone(),
            endpoint: config.endpoint.clone(),
            traces_endpoint: config.traces_endpoint.clone(),
            headers: config.headers.clone(),
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        })
    }
}

// Metrics ignore traces_endpoint and always speak protobuf; an unrecognized kind prints to the console.
pub fn metric_sink(
    config: &OtelConfig,
    clients: &HttpClients,
) -> Result<MetricSink, ExporterError> {
    let headers = config.headers.as_deref();
    match classify(&config.exporter) {
        Kind::HttpJson | Kind::HttpProto => {
            let endpoint = signal_endpoint(config.endpoint.as_deref(), Signal::Metrics);
            let builder = opentelemetry_otlp::MetricExporter::builder().with_http();
            let configured = http_exporter(
                builder,
                Protocol::HttpBinary,
                endpoint,
                headers,
                &clients.metrics,
            );
            Ok(MetricSink::Otlp(Box::new(configured.build()?)))
        }
        Kind::Grpc => {
            let builder = opentelemetry_otlp::MetricExporter::builder().with_tonic();
            let configured = grpc_exporter(builder, config.endpoint.as_deref(), headers)?;
            Ok(MetricSink::Otlp(Box::new(configured.build()?)))
        }
        Kind::Console | Kind::InMemory | Kind::Unrecognized => {
            Ok(MetricSink::Console(Box::default()))
        }
    }
}

pub fn log_sink(config: &OtelConfig, clients: &HttpClients) -> Result<LogSink, ExporterError> {
    let headers = config.headers.as_deref();
    match classify(&config.exporter) {
        Kind::InMemory => Ok(LogSink::InMemory(InMemoryLogExporter::default())),
        Kind::HttpJson | Kind::HttpProto => {
            let endpoint = signal_endpoint(config.endpoint.as_deref(), Signal::Logs);
            let builder = opentelemetry_otlp::LogExporter::builder().with_http();
            let configured = http_exporter(
                builder,
                Protocol::HttpBinary,
                endpoint,
                headers,
                &clients.logs,
            );
            Ok(LogSink::Otlp(Box::new(configured.build()?)))
        }
        Kind::Grpc => {
            let builder = opentelemetry_otlp::LogExporter::builder().with_tonic();
            let configured = grpc_exporter(builder, config.endpoint.as_deref(), headers)?;
            Ok(LogSink::Otlp(Box::new(configured.build()?)))
        }
        Kind::Console | Kind::Unrecognized => Ok(LogSink::Console(
            opentelemetry_stdout::LogExporter::default(),
        )),
    }
}

#[path = "exporters/tests.rs"]
pub(super) mod tests;
