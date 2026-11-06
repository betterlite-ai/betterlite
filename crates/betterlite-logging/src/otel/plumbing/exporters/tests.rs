#![cfg(test)]
use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use http::HeaderMap;
use opentelemetry::Context;
use opentelemetry::trace::{
    Link, Span as _, SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
    Tracer as _, TracerProvider as _,
};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use rstest::rstest;
use serde::Deserialize;
use tokio::time::sleep;
use tonic::Request as GrpcRequest;
use tonic::service::Interceptor as _;

use super::header_rewrite::{MetadataInterceptor, configured_headers};
use super::{
    ExporterError, ExporterFactories, ExporterFactory, HttpClients, Transport, build_log_exporter,
    configured_endpoint, metric_sink,
};
use crate::otel::config::ExporterSpec;
use crate::otel::plumbing::processors::processor_for;
use crate::otel::plumbing::sinks::{LogSink, MetricSink, SpanSink};

#[derive(Debug)]
pub(crate) struct Recorded {
    pub(crate) uri: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct RecordingClient {
    requests: Mutex<Vec<Recorded>>,
}

impl RecordingClient {
    pub(crate) fn recorded(&self) -> Vec<Recorded> {
        mem::take(&mut *self.requests.lock().expect("recording lock"))
    }

    pub(crate) async fn wait_for_request(&self, timeout: Duration) -> Option<Recorded> {
        let deadline = Instant::now().checked_add(timeout)?;
        loop {
            let recorded = self.recorded();
            if !recorded.is_empty() {
                return recorded.into_iter().next();
            }
            if Instant::now() >= deadline {
                return None;
            }
            sleep(Duration::from_millis(25)).await;
        }
    }
}

#[async_trait::async_trait]
impl HttpClient for RecordingClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        self.requests
            .lock()
            .expect("recording lock")
            .push(Recorded {
                uri: request.uri().to_string(),
                headers: request.headers().clone(),
                body: request.body().to_vec(),
            });
        Ok(Response::new(Bytes::new()))
    }
}

pub(crate) fn clients(recorder: &Arc<RecordingClient>) -> HttpClients {
    let concrete = Arc::clone(recorder);
    let shared: Arc<dyn HttpClient> = concrete;
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn spec(kind: &str) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    }
}

pub(crate) fn config(kind: &str) -> OtelConfig {
    OtelConfig {
        exporter: kind.to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        service_name: "litellm-proxy".to_owned(),
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

fn span_label(sink: &SpanSink) -> &'static str {
    match sink {
        SpanSink::Console(_) => "console",
        SpanSink::InMemory(_) => "in_memory",
        SpanSink::Otlp(_) => "otlp",
    }
}

fn metric_label(sink: &MetricSink) -> &'static str {
    match sink {
        MetricSink::Console(_) => "console",
        MetricSink::Otlp(_) => "otlp",
    }
}

fn log_label(sink: &LogSink) -> &'static str {
    match sink {
        LogSink::Console(_) => "console",
        LogSink::InMemory(_) => "in_memory",
        LogSink::Otlp(_) => "otlp",
    }
}

fn factories_with(
    custom: Vec<(&str, ExporterFactory)>,
) -> (ExporterFactories, Arc<RecordingClient>) {
    let recorder = Arc::new(RecordingClient::default());
    let registered = custom
        .into_iter()
        .map(|(kind, factory)| (kind.to_owned(), factory))
        .collect();
    (
        ExporterFactories::new(clients(&recorder), registered),
        recorder,
    )
}

fn factories() -> (ExporterFactories, Arc<RecordingClient>) {
    factories_with(Vec::new())
}

#[rstest]
#[case::empty("", "console")]
#[case::console("console", "console")]
#[case::console_upper("CONSOLE", "console")]
#[case::in_memory("in_memory", "in_memory")]
#[case::inmemory("InMemory", "in_memory")]
#[case::memory("memory", "in_memory")]
#[case::otlp_http("otlp_http", "otlp")]
#[case::http("http", "otlp")]
#[case::http_protobuf("http/protobuf", "otlp")]
#[case::http_json("http/json", "otlp")]
#[case::otlp_http_upper("OTLP_HTTP", "otlp")]
fn exporter_kind_builds_the_matching_sink(#[case] kind: &str, #[case] expected: &str) {
    let (factories, _recorder) = factories();
    let sink = factories.exporter_from_spec(&spec(kind)).expect("sink");
    assert_eq!(span_label(&sink), expected);
}

#[rstest]
#[case::otlp_grpc("otlp_grpc")]
#[case::grpc("grpc")]
#[case::grpc_upper("GRPC")]
#[tokio::test]
async fn exporter_grpc_kind_builds_an_otlp_sink(#[case] kind: &str) {
    let (factories, _recorder) = factories();
    let mut grpc = spec(kind);
    grpc.endpoint = Some("http://localhost:4317".to_owned());
    grpc.headers = Some("x-api-key=secret".to_owned());
    let sink = factories.exporter_from_spec(&grpc).expect("sink");
    assert_eq!(span_label(&sink), "otlp");
}

#[tokio::test]
async fn exporter_grpc_rejects_an_invalid_header_value() {
    let (factories, _recorder) = factories();
    let mut grpc = spec("grpc");
    grpc.headers = Some("x-key=line%0Abreak".to_owned());
    let error = factories
        .exporter_from_spec(&grpc)
        .expect_err("invalid header value");
    assert!(matches!(error, ExporterError::HeaderValue(_)));
}

#[rstest]
#[case::absent(None, None)]
#[case::blank(Some(""), None)]
#[case::value(Some("http://collector:4318"), Some("http://collector:4318"))]
fn exporter_endpoint_treats_a_blank_as_unset(
    #[case] endpoint: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(configured_endpoint(endpoint), expected);
}

#[test]
fn otlp_grpc_headers_are_decoded_into_metadata() {
    let headers = configured_headers(Some("authorization=Basic%20dG9rZW4%3D")).expect("headers");
    let mut interceptor = MetadataInterceptor { headers };
    let request = interceptor.call(GrpcRequest::new(())).expect("intercepted");
    let value = request
        .metadata()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(value, Some("Basic dG9rZW4="));
}

#[test]
fn otlp_grpc_metadata_interceptor_drops_the_environment_headers() {
    let headers = configured_headers(Some("authorization=Bearer tenant")).expect("headers");
    let mut interceptor = MetadataInterceptor { headers };
    let mut request = GrpcRequest::new(());
    request.metadata_mut().insert(
        "authorization",
        "Bearer operator".parse().expect("metadata"),
    );
    request
        .metadata_mut()
        .insert("x-env", "1".parse().expect("metadata"));
    let intercepted = interceptor.call(request).expect("intercepted");
    let authorization = intercepted
        .metadata()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer tenant"));
    assert!(intercepted.metadata().get("x-env").is_none());
}

#[test]
fn otlp_http_environment_headers_apply_when_the_spec_sets_none() {
    let mut http = spec("otlp_http");
    http.endpoint = Some("http://collector:4318".to_owned());
    let posted = temp_env::with_vars(
        [
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                Some("authorization=Bearer operator"),
            ),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", None),
        ],
        || post_one_span(&http),
    );
    let authorization = posted
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer operator"));
}

#[test]
fn otlp_http_configured_headers_replace_the_environment_headers() {
    let mut http = spec("otlp_http");
    http.endpoint = Some("http://collector:4318".to_owned());
    http.headers = Some("authorization=Bearer tenant".to_owned());
    let posted = temp_env::with_vars(
        [
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                Some("authorization=Bearer operator,x-env=1"),
            ),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", None),
        ],
        || post_one_span(&http),
    );
    let authorization = posted
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer tenant"));
    assert!(posted.headers.get("x-env").is_none());
}

#[test]
fn exporter_http_rejects_an_invalid_header_value() {
    let (factories, _recorder) = factories();
    let mut http = spec("otlp_http");
    http.endpoint = Some("http://collector:4318".to_owned());
    http.headers = Some("x-key=line%0Abreak".to_owned());
    let error = factories
        .exporter_from_spec(&http)
        .expect_err("invalid header value");
    assert!(matches!(error, ExporterError::HeaderValue(_)));
}

#[rstest]
#[case::unknown("jaeger")]
#[case::typo("otlp_htp")]
#[case::zipkin("zipkin")]
fn exporter_unrecognized_kind_is_rejected(#[case] kind: &str) {
    let (factories, _recorder) = factories();
    let error = factories
        .exporter_from_spec(&spec(kind))
        .expect_err("unrecognized kind");
    assert!(matches!(&error, ExporterError::UnknownKind(text) if text == kind));
}

#[test]
fn exporter_registered_factory_serves_a_custom_kind() {
    let in_memory: ExporterFactory =
        Arc::new(|_spec| Ok(SpanSink::InMemory(InMemorySpanExporter::default())));
    let (factories, _recorder) = factories_with(vec![("Custom", in_memory)]);
    let sink = factories.exporter_from_spec(&spec("custom")).expect("sink");
    assert_eq!(span_label(&sink), "in_memory");
}

#[test]
fn exporter_registered_factory_overrides_a_builtin_kind() {
    let in_memory: ExporterFactory =
        Arc::new(|_spec| Ok(SpanSink::InMemory(InMemorySpanExporter::default())));
    let (factories, _recorder) = factories_with(vec![("console", in_memory)]);
    let sink = factories
        .exporter_from_spec(&spec("console"))
        .expect("sink");
    assert_eq!(span_label(&sink), "in_memory");
}

#[test]
fn exporter_empty_kind_folds_to_console_before_the_factory_lookup() {
    let in_memory: ExporterFactory =
        Arc::new(|_spec| Ok(SpanSink::InMemory(InMemorySpanExporter::default())));
    let (factories, _recorder) = factories_with(vec![("console", in_memory)]);
    let sink = factories.exporter_from_spec(&spec("")).expect("sink");
    assert_eq!(span_label(&sink), "in_memory");
}

#[test]
fn exporter_registered_factory_error_is_returned() {
    let failing: ExporterFactory =
        Arc::new(|spec| Err(ExporterError::UnknownKind(spec.kind.clone())));
    let (factories, _recorder) = factories_with(vec![("broken", failing)]);
    let error = factories
        .exporter_from_spec(&spec("broken"))
        .expect_err("factory error");
    assert!(matches!(&error, ExporterError::UnknownKind(text) if text == "broken"));
}

#[rstest]
#[case::otlp_http("otlp_http", Transport::Http)]
#[case::http_json("http/json", Transport::Http)]
#[case::http_protobuf("http/protobuf", Transport::Http)]
#[case::grpc("otlp_grpc", Transport::Grpc)]
#[case::grpc_short("GRPC", Transport::Grpc)]
#[case::console("console", Transport::Headerless)]
#[case::in_memory("in_memory", Transport::Headerless)]
#[case::unknown("jaeger", Transport::Headerless)]
fn exporter_transport_follows_the_kind(#[case] kind: &str, #[case] expected: Transport) {
    let (factories, _recorder) = factories();
    assert_eq!(factories.exporter_transport(kind), expected);
}

#[test]
fn exporter_transport_of_a_registered_kind_carries_headers() {
    let in_memory: ExporterFactory =
        Arc::new(|_spec| Ok(SpanSink::InMemory(InMemorySpanExporter::default())));
    let (factories, _recorder) = factories_with(vec![("custom", in_memory)]);
    assert_eq!(factories.exporter_transport("Custom"), Transport::Http);
}

#[test]
fn exporter_transport_of_a_registered_grpc_kind_carries_headers() {
    let in_memory: ExporterFactory =
        Arc::new(|_spec| Ok(SpanSink::InMemory(InMemorySpanExporter::default())));
    let (factories, _recorder) = factories_with(vec![("grpc", in_memory)]);
    assert_eq!(factories.exporter_transport("grpc"), Transport::Http);
}

#[test]
fn exporter_shorthand_fields_build_the_span_exporter() {
    let (factories, _recorder) = factories();
    let sink = factories
        .build_span_exporter(&config("in_memory"))
        .expect("sink");
    assert_eq!(span_label(&sink), "in_memory");
    let error = factories
        .build_span_exporter(&config("jaeger"))
        .expect_err("unrecognized kind");
    assert!(matches!(error, ExporterError::UnknownKind(_)));
}

#[rstest]
#[case::console("console", "console")]
#[case::empty("", "console")]
#[case::in_memory("in_memory", "console")]
#[case::unknown("jaeger", "console")]
#[case::http("otlp_http", "otlp")]
#[case::http_json("http/json", "otlp")]
fn exporter_metric_sink_follows_the_kind(#[case] kind: &str, #[case] expected: &str) {
    let (_factories, recorder) = factories();
    let sink = metric_sink(&config(kind), &clients(&recorder)).expect("sink");
    assert_eq!(metric_label(&sink), expected);
}

#[tokio::test]
async fn exporter_metric_sink_builds_for_grpc() {
    let (_factories, recorder) = factories();
    let sink = metric_sink(&config("grpc"), &clients(&recorder)).expect("sink");
    assert_eq!(metric_label(&sink), "otlp");
}

#[rstest]
#[case::console("console", "console")]
#[case::unknown("jaeger", "console")]
#[case::in_memory("in_memory", "in_memory")]
#[case::http("otlp_http", "otlp")]
#[case::http_json("http/json", "otlp")]
fn exporter_log_sink_follows_the_kind(#[case] kind: &str, #[case] expected: &str) {
    let (_factories, recorder) = factories();
    let sink = build_log_exporter(&config(kind), &clients(&recorder)).expect("sink");
    assert_eq!(log_label(&sink), expected);
}

#[tokio::test]
async fn exporter_log_sink_builds_for_grpc() {
    let (_factories, recorder) = factories();
    let sink = build_log_exporter(&config("grpc"), &clients(&recorder)).expect("sink");
    assert_eq!(log_label(&sink), "otlp");
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "resourceSpans")]
    resource_spans: Vec<ResourceSpans>,
}

#[derive(Deserialize)]
struct ResourceSpans {
    #[serde(rename = "scopeSpans")]
    scope_spans: Vec<ScopeSpans>,
}

#[derive(Deserialize)]
struct ScopeSpans {
    spans: Vec<WireSpan>,
}

#[derive(Deserialize)]
struct WireSpan {
    #[serde(rename = "traceId")]
    trace_id: String,
    #[serde(rename = "spanId")]
    span_id: String,
    #[serde(rename = "parentSpanId", default)]
    parent_span_id: String,
    name: String,
    kind: i32,
    #[serde(default)]
    links: Vec<WireLink>,
}

#[derive(Deserialize)]
struct WireLink {
    #[serde(rename = "traceId")]
    trace_id: String,
    #[serde(rename = "spanId")]
    span_id: String,
}

fn post_one_span(spec: &ExporterSpec) -> Recorded {
    let (factories, recorder) = factories();
    let sink = factories.exporter_from_spec(spec).expect("sink");
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor_for(sink, Some(true)))
        .build();
    provider.tracer("codec").start("operation").end();
    provider.shutdown().expect("shutdown");
    let mut posted = recorder.recorded();
    assert_eq!(posted.len(), 1);
    posted.remove(0)
}

fn content_type(posted: &Recorded) -> &str {
    posted
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

#[test]
fn codec_json_kind_hex_encodes_parent_and_link_ids() {
    let (factories, recorder) = factories();
    let mut json = spec("http/json");
    json.endpoint = Some("http://collector:4318".to_owned());
    let sink = factories.exporter_from_spec(&json).expect("sink");
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor_for(sink, Some(true)))
        .build();
    let remote = SpanContext::new(
        TraceId::from(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128),
        SpanId::from(0x1112_1314_1516_1718_u64),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let parent = Context::new().with_remote_span_context(remote.clone());
    let tracer = provider.tracer("codec");
    let child = tracer
        .span_builder("child")
        .with_links(vec![Link::with_context(remote)]);
    tracer.build_with_context(child, &parent).end();
    provider.shutdown().expect("shutdown");
    let posted = recorder.recorded();
    let payload: Payload = serde_json::from_slice(&posted[0].body).expect("json body");
    let wire = &payload.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(wire.parent_span_id, "1112131415161718");
    assert_eq!(wire.trace_id, "0102030405060708090a0b0c0d0e0f10");
    assert_eq!(wire.links[0].trace_id, "0102030405060708090a0b0c0d0e0f10");
    assert_eq!(wire.links[0].span_id, "1112131415161718");
}

#[test]
fn codec_json_kind_posts_ids_as_hex_and_enums_as_integers() {
    let mut json = spec("http/json");
    json.endpoint = Some("http://collector:4318".to_owned());
    let posted = post_one_span(&json);
    assert_eq!(content_type(&posted), "application/json");
    let payload: Payload = serde_json::from_slice(&posted.body).expect("json body");
    let wire = &payload.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(wire.name, "operation");
    assert_eq!(wire.trace_id.len(), 32);
    assert_eq!(wire.span_id.len(), 16);
    assert!(wire.trace_id.chars().all(|digit| digit.is_ascii_hexdigit()));
    assert!(wire.span_id.chars().all(|digit| digit.is_ascii_hexdigit()));
    assert_eq!(wire.kind, 1_i32);
}

#[rstest]
#[case::otlp_http("otlp_http")]
#[case::http("http")]
#[case::http_protobuf("http/protobuf")]
fn codec_protobuf_kinds_post_the_binary_content_type(#[case] kind: &str) {
    let mut protobuf = spec(kind);
    protobuf.endpoint = Some("http://collector:4318".to_owned());
    let posted = post_one_span(&protobuf);
    assert_eq!(content_type(&posted), "application/x-protobuf");
    assert!(serde_json::from_slice::<Payload>(&posted.body).is_err());
}

#[rstest]
#[case::base("http://collector:4318", None, "http://collector:4318/v1/traces")]
#[case::trailing_slash("http://collector:4318/", None, "http://collector:4318/v1/traces")]
#[case::from_logs_path("http://collector/v1/logs", None, "http://collector/v1/traces")]
#[case::splunk_untouched(
    "https://ingest.signalfx.com/v2/trace/otlp",
    None,
    "https://ingest.signalfx.com/v2/trace/otlp"
)]
#[case::traces_endpoint_wins(
    "http://collector:4318",
    Some("http://other/custom/path"),
    "http://other/custom/path"
)]
#[case::blank_traces_endpoint("http://collector:4318", Some(""), "http://collector:4318/v1/traces")]
fn otlp_span_endpoint_is_rewritten_per_signal(
    #[case] endpoint: &str,
    #[case] traces_endpoint: Option<&str>,
    #[case] expected: &str,
) {
    let mut json = spec("http/json");
    json.endpoint = Some(endpoint.to_owned());
    json.traces_endpoint = traces_endpoint.map(str::to_owned);
    assert_eq!(post_one_span(&json).uri, expected);
}

#[rstest]
#[case::plain("x-api-key=abc", "x-api-key", "abc")]
#[case::percent_decoded("authorization=Basic%20dG9rZW4%3D", "authorization", "Basic dG9rZW4=")]
#[case::decoded_once("x-token=50%2525", "x-token", "50%25")]
#[case::name_lowercased("X-Key=v", "x-key", "v")]
fn otlp_headers_reach_the_request_decoded_once(
    #[case] raw: &str,
    #[case] name: &str,
    #[case] expected: &str,
) {
    let mut json = spec("http/json");
    json.endpoint = Some("http://collector:4318".to_owned());
    json.headers = Some(raw.to_owned());
    let posted = post_one_span(&json);
    let value = posted
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok());
    assert_eq!(value, Some(expected));
}
