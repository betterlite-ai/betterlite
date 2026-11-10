#![cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::HeaderMap;
use http::header::CONTENT_TYPE;
use opentelemetry::InstrumentationScope;
use opentelemetry::logs::{Logger as _, LoggerProvider as _};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::logs::{SdkLoggerProvider, SimpleLogProcessor};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use rstest::rstest;
use temp_env::with_vars;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::LegacyExporterBuilder;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::selection::{
    log_exporter_plan, metric_reader_plan, span_processor_plan,
};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel::plumbing::processors::ExportProcessor;
use crate::otel::plumbing::sinks::LogSink;

#[derive(Debug, Clone)]
struct Recorded {
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Debug, Default)]
struct LastRequest(Mutex<Option<Recorded>>);

impl LastRequest {
    fn take(&self) -> Recorded {
        self.0
            .lock()
            .expect("request lock")
            .take()
            .expect("a recorded request")
    }

    fn recorded(&self) -> bool {
        self.0.lock().expect("request lock").is_some()
    }
}

#[async_trait::async_trait]
impl HttpClient for LastRequest {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let recorded = Recorded {
            uri: request.uri().to_string(),
            headers: request.headers().clone(),
            body: request.body().to_vec(),
        };
        *self.0.lock().expect("request lock") = Some(recorded);
        Ok(Response::new(Bytes::new()))
    }
}

fn clients(recorder: &Arc<LastRequest>) -> HttpClients {
    let concrete = Arc::clone(recorder);
    let shared: Arc<dyn HttpClient> = concrete;
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn config(
    exporter: Exporter,
    endpoint: Option<&str>,
    headers: Option<&str>,
) -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter,
        endpoint: endpoint.map(str::to_owned),
        headers: headers.map(str::to_owned),
        signals: Signals::default(),
        service_name: "litellm".to_owned(),
        deployment_environment: "production".to_owned(),
        model_id: "litellm".to_owned(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn named(text: &str) -> Exporter {
    Exporter::Named(text.to_owned())
}

fn builder(recorder: &Arc<LastRequest>) -> LegacyExporterBuilder {
    LegacyExporterBuilder::new(clients(recorder), InMemorySpanExporter::default())
}

fn export_one_span(processor: ExportProcessor) {
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    provider.tracer("build-test").start("operation").end();
    provider.shutdown().expect("shutdown");
}

fn header<'digest>(recorded: &'digest Recorded, name: &str) -> Option<&'digest str> {
    let value = recorded.headers.get(name)?;
    value.to_str().ok()
}

#[test]
fn shared_plan_exports_into_the_callers_in_memory_exporter() {
    let recorder = Arc::new(LastRequest::default());
    let shared = InMemorySpanExporter::default();
    let plan = span_processor_plan(&config(Exporter::InMemory, None, None), None, None, None)
        .expect("plan");
    let processor = LegacyExporterBuilder::new(clients(&recorder), shared.clone())
        .span_processor(&plan)
        .expect("processor");
    assert!(matches!(processor, ExportProcessor::Simple(_)));
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    provider.tracer("build-test").start("operation").end();
    let spans = shared.get_finished_spans().expect("spans");
    assert_eq!(spans.len(), 1);
}

#[rstest]
#[case::console(Exporter::Console)]
#[case::unknown_text(named("jaeger"))]
fn console_plan_builds_a_batch_processor(#[case] exporter: Exporter) {
    let recorder = Arc::new(LastRequest::default());
    let plan = span_processor_plan(&config(exporter, None, None), None, None, None).expect("plan");
    let processor = builder(&recorder).span_processor(&plan).expect("processor");
    assert!(matches!(processor, ExportProcessor::Batch(_)));
}

#[test]
fn otlp_http_plan_sends_to_the_normalized_endpoint_with_the_plan_headers() {
    let recorder = Arc::new(LastRequest::default());
    let plan = span_processor_plan(
        &config(
            named("otlp_http"),
            Some("http://collector:4318/v1/logs"),
            Some("x-api-key=abc,x-team=one"),
        ),
        None,
        None,
        None,
    )
    .expect("plan");
    let processor = builder(&recorder).span_processor(&plan).expect("processor");
    export_one_span(processor);
    let posted = recorder.take();
    assert_eq!(posted.uri, "http://collector:4318/v1/traces");
    assert_eq!(header(&posted, "x-api-key"), Some("abc"));
    assert_eq!(header(&posted, "x-team"), Some("one"));
}

#[rstest]
#[case::langtrace(
    "http://collector/api/trace",
    Some("langtrace"),
    "http://collector/api/trace"
)]
#[case::v1_base("http://collector/v1", None, "http://collector/v1/traces")]
fn the_normalized_trace_path_reaches_the_exporter_verbatim(
    #[case] endpoint: &str,
    #[case] callback_name: Option<&str>,
    #[case] expected: &str,
) {
    let recorder = Arc::new(LastRequest::default());
    let plan = span_processor_plan(
        &config(named("otlp_http"), Some(endpoint), None),
        None,
        None,
        callback_name,
    )
    .expect("plan");
    let processor = builder(&recorder).span_processor(&plan).expect("processor");
    export_one_span(processor);
    let posted = recorder.take();
    assert_eq!(posted.uri, expected);
}

#[test]
fn otlp_http_plan_posts_protobuf_for_the_json_http_name() {
    let recorder = Arc::new(LastRequest::default());
    let plan = span_processor_plan(
        &config(named("http/json"), Some("http://collector:4318"), None),
        None,
        None,
        None,
    )
    .expect("plan");
    let processor = builder(&recorder).span_processor(&plan).expect("processor");
    export_one_span(processor);
    let posted = recorder.take();
    assert_eq!(
        header(&posted, CONTENT_TYPE.as_str()),
        Some("application/x-protobuf")
    );
    assert_ne!(posted.body.first(), Some(&b'{'));
}

#[rstest]
#[case::grpc(named("grpc"))]
#[case::otlp_grpc(named("otlp_grpc"))]
fn an_otlp_grpc_plan_builds_the_grpc_exporter(#[case] exporter: Exporter) {
    let recorder = Arc::new(LastRequest::default());
    // A console sink ignores the endpoint; both OTLP builders reject this malformed one at build.
    let plan = span_processor_plan(
        &config(exporter, Some("http://[invalid"), None),
        None,
        None,
        None,
    )
    .expect("plan");
    let error = builder(&recorder).span_processor(&plan).unwrap_err();
    assert!(matches!(error, LegacyExporterError::Exporter(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_otlp_grpc_plan_speaks_http2_to_its_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let recorder = Arc::new(LastRequest::default());
    let plan = span_processor_plan(
        &config(named("grpc"), Some(&endpoint), None),
        None,
        None,
        None,
    )
    .expect("plan");
    // The exporter reads this at build, so a silent collector ends the export after a second.
    let processor = with_vars(
        [("OTEL_EXPORTER_OTLP_TRACES_TIMEOUT", Some("1000"))],
        || builder(&recorder).span_processor(&plan).expect("processor"),
    );
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    provider.tracer("build-test").start("operation").end();
    // This blocks one worker while the other drives the connection and the export timeout.
    let _flushed = provider.force_flush();
    let (mut stream, _peer) = timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("the gRPC exporter must connect")
        .expect("accept");
    let mut preface = [0_u8; 24];
    timeout(Duration::from_secs(10), stream.read_exact(&mut preface))
        .await
        .expect("the HTTP/2 preface must arrive")
        .expect("read");
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    assert!(
        !recorder.recorded(),
        "the gRPC plan must not send through the HTTP client"
    );
}

#[rstest]
#[case::shared(Exporter::InMemory, "in_memory")]
#[case::console(Exporter::Console, "console")]
#[case::unknown_text(named("jaeger"), "console")]
fn log_exporter_follows_the_plan(#[case] exporter: Exporter, #[case] expected: &str) {
    let recorder = Arc::new(LastRequest::default());
    let plan =
        log_exporter_plan(&config(exporter, None, None), None, &BTreeMap::new()).expect("plan");
    let sink = builder(&recorder).log_exporter(plan).expect("sink");
    let label = match sink {
        LogSink::Console(_) => "console",
        LogSink::InMemory(_) => "in_memory",
        LogSink::Otlp(_) => "otlp",
    };
    assert_eq!(label, expected);
}

#[test]
fn log_exporter_posts_to_the_normalized_endpoint_with_the_plan_headers() {
    let recorder = Arc::new(LastRequest::default());
    let plan = log_exporter_plan(
        &config(
            named("otlp_http"),
            Some("http://collector:4318/v1/traces"),
            Some("x-api-key=abc"),
        ),
        None,
        &BTreeMap::new(),
    )
    .expect("plan");
    let sink = builder(&recorder).log_exporter(plan).expect("sink");
    let provider = SdkLoggerProvider::builder()
        .with_log_processor(SimpleLogProcessor::new(sink))
        .build();
    let logger = provider.logger_with_scope(InstrumentationScope::builder("build-test").build());
    logger.emit(logger.create_log_record());
    let posted = recorder.take();
    assert_eq!(posted.uri, "http://collector:4318/v1/logs");
    assert_eq!(header(&posted, "x-api-key"), Some("abc"));
}

#[rstest]
#[case::console(Exporter::Console)]
#[case::in_memory(Exporter::InMemory)]
fn metric_reader_builds_for_console_plans(#[case] exporter: Exporter) {
    let recorder = Arc::new(LastRequest::default());
    let plan = metric_reader_plan(&config(exporter, Some("http://collector:4318"), None), None)
        .expect("plan");
    let reader = builder(&recorder).metric_reader(plan).expect("reader");
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    provider
        .meter("build-test")
        .u64_counter("probe")
        .build()
        .add(1, &[]);
    provider.force_flush().expect("flush");
    assert!(
        !recorder.recorded(),
        "a console metric reader must not post to the configured endpoint"
    );
}

#[test]
fn metric_reader_posts_to_the_normalized_metrics_endpoint() {
    let recorder = Arc::new(LastRequest::default());
    let plan = metric_reader_plan(
        &config(
            named("otlp_http"),
            Some("http://collector:4318/v1/traces"),
            Some("x-api-key=abc"),
        ),
        None,
    )
    .expect("plan");
    let reader = builder(&recorder).metric_reader(plan).expect("reader");
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    provider
        .meter("build-test")
        .u64_counter("probe")
        .build()
        .add(1, &[]);
    provider.force_flush().expect("flush");
    let posted = recorder.take();
    assert_eq!(posted.uri, "http://collector:4318/v1/metrics");
    assert_eq!(header(&posted, "x-api-key"), Some("abc"));
}
