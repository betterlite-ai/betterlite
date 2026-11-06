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

pub type ExporterFactory = Arc<dyn Fn(&ExporterSpec) -> ExportResult<SpanSink> + Send + Sync>;

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

pub type ExportResult<T> = Result<T, ExporterError>;

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

// LiteLLM folds an empty kind into "console" before consulting a registered factory.
fn resolved_kind(kind: &str) -> String {
    match kind.to_lowercase().as_str() {
        "" => "console".to_owned(),
        lowered => lowered.to_owned(),
    }
}

// An empty endpoint is unset, as the SDK's own endpoint handling treats it.
fn configured_endpoint(endpoint: Option<&str>) -> Option<&str> {
    match endpoint {
        Some("") | None => None,
        Some(url) => Some(url),
    }
}

// The SDK percent-decodes header values it is given, and parse_headers already did.
fn http_exporter<B>(
    builder: B,
    protocol: Protocol,
    endpoint: Option<&str>,
    raw_headers: Option<&str>,
    client: &Arc<dyn HttpClient>,
) -> ExportResult<B>
where
    B: WithExportConfig + WithHttpConfig,
{
    let mut headers: HashMap<String, String> = HashMap::new();
    for (name, value) in parse_headers(raw_headers) {
        HeaderName::from_bytes(name.as_bytes())?;
        HeaderValue::from_str(&value)?;
        headers.insert(name, value.replace('%', "%25"));
    }
    let configured = builder
        .with_protocol(protocol)
        .with_shared_http_client(Arc::clone(client))
        .with_headers(headers);
    Ok(match configured_endpoint(endpoint) {
        Some(url) => configured.with_endpoint(url),
        None => configured,
    })
}

fn metadata_from_headers(raw_headers: Option<&str>) -> ExportResult<MetadataMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in parse_headers(raw_headers) {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(&value)?,
        );
    }
    Ok(MetadataMap::from_headers(headers))
}

fn grpc_exporter<B>(
    builder: B,
    endpoint: Option<&str>,
    raw_headers: Option<&str>,
) -> ExportResult<B>
where
    B: WithExportConfig + WithTonicConfig,
{
    let configured = builder.with_metadata(metadata_from_headers(raw_headers)?);
    Ok(match configured_endpoint(endpoint) {
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

    // A registered factory is assumed to build an OTLP exporter, as LiteLLM documents.
    #[must_use]
    pub fn exporter_transport(&self, kind: &str) -> Transport {
        if self.factories.contains_key(&kind.to_lowercase()) {
            return Transport::Http;
        }
        match classify(kind) {
            Kind::HttpJson | Kind::HttpProto => Transport::Http,
            Kind::Grpc => Transport::Grpc,
            Kind::Console | Kind::InMemory | Kind::Unrecognized => Transport::Headerless,
        }
    }

    pub fn exporter_from_spec(&self, spec: &ExporterSpec) -> ExportResult<SpanSink> {
        let resolved = resolved_kind(&spec.kind);
        if let Some(factory) = self.factories.get(&resolved) {
            return factory(spec);
        }
        let headers = spec.headers.as_deref();
        match classify(&resolved) {
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
                let endpoint = configured_endpoint(spec.traces_endpoint.as_deref())
                    .map(str::to_owned)
                    .or_else(|| signal_endpoint(spec.endpoint.as_deref(), Signal::Traces));
                let builder = opentelemetry_otlp::SpanExporter::builder().with_http();
                let configured = http_exporter(
                    builder,
                    protocol,
                    endpoint.as_deref(),
                    headers,
                    &self.clients.traces,
                )?;
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
    pub fn build_span_exporter(&self, config: &OtelConfig) -> ExportResult<SpanSink> {
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

// Metrics and logs build their OTLP exporters the same way and only differ in the types and the fallback arms.
macro_rules! signal_sink {
    ($exporter:ident => $sink:ident, $config:expr, $client:expr, $signal:expr, { $($fallback:tt)* }) => {{
        let headers = $config.headers.as_deref();
        match classify(&$config.exporter) {
            Kind::HttpJson | Kind::HttpProto => {
                let endpoint = signal_endpoint($config.endpoint.as_deref(), $signal);
                let builder = opentelemetry_otlp::$exporter::builder().with_http();
                let configured = http_exporter(
                    builder,
                    Protocol::HttpBinary,
                    endpoint.as_deref(),
                    headers,
                    $client,
                )?;
                Ok($sink::Otlp(Box::new(configured.build()?)))
            }
            Kind::Grpc => {
                let builder = opentelemetry_otlp::$exporter::builder().with_tonic();
                let configured = grpc_exporter(builder, $config.endpoint.as_deref(), headers)?;
                Ok($sink::Otlp(Box::new(configured.build()?)))
            }
            $($fallback)*
        }
    }};
}

// Metrics ignore traces_endpoint and always speak protobuf; an unrecognized kind prints to the console.
pub fn metric_sink(config: &OtelConfig, clients: &HttpClients) -> ExportResult<MetricSink> {
    signal_sink!(MetricExporter => MetricSink, config, &clients.metrics, Signal::Metrics, {
        Kind::Console | Kind::InMemory | Kind::Unrecognized => Ok(MetricSink::Console(Box::default())),
    })
}

pub fn build_log_exporter(config: &OtelConfig, clients: &HttpClients) -> ExportResult<LogSink> {
    signal_sink!(LogExporter => LogSink, config, &clients.logs, Signal::Logs, {
        Kind::InMemory => Ok(LogSink::InMemory(InMemoryLogExporter::default())),
        Kind::Console | Kind::Unrecognized => Ok(LogSink::Console(opentelemetry_stdout::LogExporter::default())),
    })
}

#[path = "exporters/tests.rs"]
pub(super) mod tests;
