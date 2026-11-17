#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_pre_post::hooks::{ApiCallLogger as _, PreCallEvent};
use betterlite_logging::otel::config::destination::{
    DestinationFields, OtelDestination, SpanScope,
};
use betterlite_logging::otel::config::{
    CaptureMessageContent, ExporterOwner, ExporterSpec, Settings,
};
use betterlite_logging::otel::emitter::span_emission::{DedupCache, SpanEmitter};
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::call_context::CallContext;
use betterlite_logging::otel::logger::v2::{
    OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource,
};
use betterlite_logging::otel::model::metadata::llm_call_event::LLMCallEvent;
use betterlite_logging::otel::model::metadata::wire::callback::{
    Callback, OptionalParams, Params, ProxyRequest,
};
use betterlite_logging::otel::model::metadata::wire::payload::Metadata;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, RequestTraceContext,
};
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use betterlite_logging::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider,
};
use betterlite_models::decisions::Json;
use chrono::FixedOffset;
use indexmap::IndexMap;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceFlags, TraceId, TraceState, TracerProvider as _,
};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SdkTracerProvider, SpanData};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Number, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer};

pub(crate) const CALLBACK: &str = "arize";
pub(crate) const ROOT_TRACE: &str = "0af7651916cd43dd8448eb211c80319c";

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(crate) fn payload<T>(call_type: &str, status: &str, metadata: T) -> StandardLoggingPayload
where
    T: Serialize,
{
    // parsed through the crate's own Json first: the lint table forbids naming serde_json's Value
    let parsed: Json = serde_json::from_str(PAYLOAD).expect("payload fixture");
    let mut value = serde_json::to_value(parsed).expect("payload fixture");
    let object = value.as_object_mut().expect("payload object");
    object.insert("call_type".to_owned(), json!(call_type));
    object.insert("status".to_owned(), json!(status));
    if status == "failure" {
        let fields = json_object(vec![
            ("llm_api_status", Json::Text("failure".to_owned())),
            ("guardrail_status", Json::Text("not_run".to_owned())),
        ]);
        object.insert(
            "status_fields".to_owned(),
            serde_json::to_value(fields).expect("status fields"),
        );
    }
    object.insert(
        "metadata".to_owned(),
        serde_json::to_value(metadata).expect("payload metadata"),
    );
    serde_json::from_value(value).expect("standard logging payload")
}

pub(crate) fn inference() -> StandardLoggingPayload {
    payload("completion", "success", json_object(Vec::new()))
}

pub(crate) fn inference_failure() -> StandardLoggingPayload {
    payload("completion", "failure", json_object(Vec::new()))
}

pub(crate) fn mcp_tool() -> StandardLoggingPayload {
    payload("call_mcp_tool", "success", tool_call_metadata())
}

pub(crate) fn mcp_tool_failure() -> StandardLoggingPayload {
    payload("call_mcp_tool", "failure", tool_call_metadata())
}

// the call id the OTel lifecycle keys its carrier and dedup markers on
pub(crate) fn with_id(mut payload: StandardLoggingPayload, id: &str) -> StandardLoggingPayload {
    id.clone_into(&mut payload.id);
    payload.litellm_call_id = Some(id.to_owned());
    payload
}

// a payload with no upstream start stamp: without a carrier, close cannot tell it ran
pub(crate) fn zero_start(mut payload: StandardLoggingPayload) -> StandardLoggingPayload {
    payload.start_time = Decimal::ZERO;
    payload
}

// a payload whose provider never stamped the first chunk
pub(crate) fn zero_completion_start(mut payload: StandardLoggingPayload) -> StandardLoggingPayload {
    payload.completion_start_time = Decimal::ZERO;
    payload
}

// a close callback whose payload carries a provider response
pub(crate) fn with_response(
    mut payload: StandardLoggingPayload,
    response: Json,
) -> StandardLoggingPayload {
    payload.response = Some(response);
    payload
}

pub(crate) fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

// the token usage a provider's response reports
pub(crate) fn token_usage(prompt_tokens: Json, completion_tokens: Json) -> Json {
    Json::Object(IndexMap::from([(
        "usage".to_owned(),
        Json::Object(IndexMap::from([
            ("prompt_tokens".to_owned(), prompt_tokens),
            ("completion_tokens".to_owned(), completion_tokens),
        ])),
    )]))
}

// the string dynamic parameters a preset reads, such as a New Relic key
pub(crate) fn text_params(pairs: &[(&str, &str)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Json::Text((*value).to_owned())))
            .collect(),
    )
}

// a Json object from its entries, as the proxy writes a preset's dynamic params and metadata
pub(crate) fn json_object(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn tool_name_metadata() -> Json {
    json_object(vec![("name", Json::Text("search".to_owned()))])
}

fn project_name_metadata() -> Json {
    json_object(vec![(
        "phoenix_project_name",
        Json::Text("proj".to_owned()),
    )])
}

fn tool_call_metadata() -> Json {
    json_object(vec![("mcp_tool_call_metadata", tool_name_metadata())])
}

pub(crate) fn mcp_list() -> StandardLoggingPayload {
    payload("list_mcp_tools", "success", json_object(Vec::new()))
}

// key/team config a Phoenix route reads; the identity is trusted, never the caller's carrier, and a tool call also names its tool
fn mcp_auth_payload(call_type: &str, tool_metadata: Option<Json>) -> StandardLoggingPayload {
    let mut entries = Vec::new();
    if let Some(metadata) = tool_metadata {
        entries.push(("mcp_tool_call_metadata", metadata));
    }
    entries.push(("user_api_key_auth_metadata", project_name_metadata()));
    payload(call_type, "success", json_object(entries))
}

pub(crate) fn mcp_tool_with_auth() -> StandardLoggingPayload {
    mcp_auth_payload("call_mcp_tool", Some(tool_name_metadata()))
}

pub(crate) fn mcp_list_with_auth() -> StandardLoggingPayload {
    mcp_auth_payload("list_mcp_tools", None)
}

pub(crate) fn mcp_list_failure() -> StandardLoggingPayload {
    payload("list_mcp_tools", "failure", json_object(Vec::new()))
}

pub(crate) fn team_payload() -> StandardLoggingPayload {
    payload(
        "completion",
        "success",
        json_object(vec![
            ("user_api_key_team_id", Json::Text("team-a".to_owned())),
            ("user_api_key_hash", Json::Text("hash".to_owned())),
        ]),
    )
}

pub(crate) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    event_with(payload, Json::Object(IndexMap::new()), IndexMap::new())
}

pub(crate) fn event_with(
    payload: StandardLoggingPayload,
    dynamic: Json,
    metadata: IndexMap<String, Json>,
) -> Arc<LoggerEvent> {
    log_event(payload, dynamic, metadata, false)
}

// a streaming close: the wire callback carries stream true, so the first-chunk time is measured
pub(crate) fn stream_event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    log_event(payload, Json::Null, IndexMap::new(), true)
}

fn log_event(
    payload: StandardLoggingPayload,
    dynamic: Json,
    metadata: IndexMap<String, Json>,
    stream: bool,
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic),
        litellm_metadata: Arc::new(metadata),
        stream,
        guardrail_snapshot: None,
    })
}

// a close callback whose wire dict carried no standard logging object: only the boundary span exists
pub(crate) fn payloadless_call(call_id: &str) -> CallContext {
    let callback = Callback {
        litellm_call_id: Some(call_id.to_owned()),
        model: Some("gpt-4o".to_owned()),
        call_type: Some("completion".to_owned()),
        standard_logging_object: None,
        dynamic_params: None,
        litellm_no_upstream_llm_call: false,
        api_call_start_time: None,
        completion_start_time: None,
        optional_params: OptionalParams { stream: false },
        litellm_params: Params {
            metadata: Metadata::default(),
            litellm_metadata: Metadata::default(),
            litellm_session_id: None,
            proxy_server_request: ProxyRequest { headers: None },
        },
    };
    CallContext {
        event: LLMCallEvent::from_dict(&callback).expect("call event"),
        dynamic_params: Arc::new(Json::Null),
        stream: false,
        api_call_start_time: Decimal::ZERO,
        start_time: Decimal::ZERO,
        end_time: Decimal::ZERO,
        completion_start_time: Decimal::ZERO,
    }
}

pub(crate) fn no_upstream() -> IndexMap<String, Json> {
    IndexMap::from([("litellm_no_upstream_llm_call".to_owned(), Json::Bool(true))])
}

pub(crate) fn creds(space: &str) -> Json {
    Json::Object(IndexMap::from([
        ("arize_space_id".to_owned(), Json::Text(space.to_owned())),
        ("arize_api_key".to_owned(), Json::Text("K".to_owned())),
    ]))
}

pub(crate) fn details(
    call_id: &str,
    model: &str,
    no_upstream: bool,
    dynamic: Option<Json>,
    auth: Option<Json>,
) -> Json {
    let mut object = IndexMap::new();
    object.insert("model".to_owned(), Json::Text(model.to_owned()));
    object.insert("call_type".to_owned(), Json::Text("completion".to_owned()));
    object.insert("litellm_call_id".to_owned(), Json::Text(call_id.to_owned()));
    object.insert(
        "litellm_no_upstream_llm_call".to_owned(),
        Json::Bool(no_upstream),
    );
    if let Some(dynamic_params) = dynamic {
        object.insert(
            "standard_callback_dynamic_params".to_owned(),
            dynamic_params,
        );
    }
    if let Some(identity) = auth {
        let metadata = Json::Object(IndexMap::from([(
            "user_api_key_auth_metadata".to_owned(),
            identity,
        )]));
        object.insert(
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([("metadata".to_owned(), metadata)])),
        );
    }
    Json::Object(object)
}

// one pre_call boundary as a test states it: the model, whether the proxy tagged a gate rejection, and the request's dynamic params and auth
pub(crate) struct PreCall<'call> {
    pub(crate) call_id: &'call str,
    pub(crate) model: &'call str,
    pub(crate) no_upstream: bool,
    pub(crate) dynamic: Option<Json>,
    pub(crate) auth: Option<Json>,
}

pub(crate) fn pre_call(logger: &OpenTelemetryV2, call: PreCall<'_>) {
    let details = details(
        call.call_id,
        call.model,
        call.no_upstream,
        call.dynamic,
        call.auth,
    );
    let messages = Json::List(Vec::new());
    logger
        .log_pre_api_call(&PreCallEvent {
            model: call.model,
            messages: &messages,
            details: &details,
        })
        .expect("pre api call");
}

pub(crate) async fn success(logger: &OpenTelemetryV2, event: Arc<LoggerEvent>) {
    <OpenTelemetryV2 as CallbackLogger>::async_log_success_event(logger, event)
        .await
        .expect("success event");
}

pub(crate) async fn failure(logger: &OpenTelemetryV2, event: Arc<LoggerEvent>) {
    <OpenTelemetryV2 as CallbackLogger>::async_log_failure_event(logger, event)
        .await
        .expect("failure event");
}

pub(crate) fn anchored() -> RequestTraceContext {
    RequestTraceContext {
        root: Some(AnchoredSpan {
            context: root_span_context(),
            http_route: Some("/v1/chat/completions".to_owned()),
        }),
        mcp_carrier: None,
        mcp_transport: None,
        destinations: Vec::new(),
    }
}

pub(crate) fn empty_trace() -> RequestTraceContext {
    RequestTraceContext::default()
}

pub(crate) fn root_trace_id() -> TraceId {
    TraceId::from_hex(ROOT_TRACE).expect("trace id")
}

pub(crate) fn root_span_id() -> SpanId {
    SpanId::from_hex("b7ad6b7169203331").expect("span id")
}

pub(crate) fn root_span_context() -> SpanContext {
    SpanContext::new(
        root_trace_id(),
        root_span_id(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    )
}

pub(crate) fn destination(callback: &str) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some(callback.to_owned()),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}

#[derive(Debug)]
pub(crate) struct FixedScope;

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

#[derive(Debug)]
pub(crate) struct FixedTrace(Mutex<RequestTraceContext>);

impl RequestTraceSource for FixedTrace {
    fn trace_context(&self) -> RequestTraceContext {
        self.0.lock().expect("trace context").clone()
    }
}

#[derive(Debug)]
pub(crate) struct NoClient;

#[async_trait::async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the lifecycle tests export in memory"))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Built(Arc<BuiltInner>);

#[derive(Default)]
struct BuiltInner {
    specs: Mutex<Vec<ExporterSpec>>,
    sinks: Mutex<Vec<InMemorySpanExporter>>,
}

impl Built {
    pub(crate) fn specs(&self) -> Vec<ExporterSpec> {
        self.0.specs.lock().expect("built specs").clone()
    }

    pub(crate) fn builds(&self) -> usize {
        self.0.specs.lock().expect("built specs").len()
    }

    pub(crate) fn sink(&self, index: usize) -> InMemorySpanExporter {
        self.0
            .sinks
            .lock()
            .expect("built sinks")
            .get(index)
            .cloned()
            .expect("a sink per built provider")
    }
}

pub(crate) fn factories(built: &Built) -> Arc<ExporterFactories> {
    let recorder = built.clone();
    let factory: ExporterFactory = Arc::new(move |spec: &ExporterSpec| -> ExportResult<SpanSink> {
        let sink = InMemorySpanExporter::default();
        recorder
            .0
            .specs
            .lock()
            .expect("built specs")
            .push(spec.clone());
        recorder
            .0
            .sinks
            .lock()
            .expect("built sinks")
            .push(sink.clone());
        Ok(SpanSink::InMemory(sink))
    });
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    Arc::new(ExporterFactories::new(
        HttpClients {
            traces: Arc::clone(&client),
            metrics: Arc::clone(&client),
            logs: client,
        },
        BTreeMap::from([("otlp_http".to_owned(), factory)]),
    ))
}

pub(crate) fn hosts() -> Arc<[AllowedDestination]> {
    Arc::from(Vec::<AllowedDestination>::new())
}

pub(crate) fn cache(
    settings: Settings,
    callback: Option<&str>,
    built: &Built,
) -> TenantTracerCache {
    let sources = TenantSources {
        factories: factories(built),
        scope_source: Arc::new(FixedScope),
        allowed_hosts: hosts(),
    };
    TenantTracerCache::new(settings, callback.map(str::to_owned), "litellm", sources)
}

pub(crate) fn default_tracer() -> SdkTracer {
    SdkTracerProvider::builder().build().tracer("litellm")
}

pub(crate) struct Harness {
    pub(crate) logger: OpenTelemetryV2,
    pub(crate) spans: InMemorySpanExporter,
    pub(crate) built: Built,
}

pub(crate) fn harness(settings: Settings, trace: RequestTraceContext) -> Harness {
    harness_with(settings, trace, None)
}

pub(crate) fn harness_with(
    settings: Settings,
    trace: RequestTraceContext,
    metrics: Option<Arc<GenAiMetricRecorder>>,
) -> Harness {
    harness_for(settings, trace, CALLBACK, metrics)
}

// a harness whose callback owns the exporters under another preset name, such as arize_phoenix
pub(crate) fn harness_for(
    settings: Settings,
    trace: RequestTraceContext,
    callback: &str,
    metrics: Option<Arc<GenAiMetricRecorder>>,
) -> Harness {
    let built = Built::default();
    let spans = InMemorySpanExporter::default();
    let factories = factories(&built);
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let tracer = main_tracer(&settings, &factories, &scope, &spans);
    let emitter = main_emitter(&tracer);
    let sources = TenantSources {
        factories,
        scope_source: Arc::clone(&scope),
        allowed_hosts: hosts(),
    };
    let tenants = TenantTracerCache::new(
        settings.clone(),
        Some(callback.to_owned()),
        "litellm",
        sources,
    );
    let source = Arc::new(FixedTrace(Mutex::new(trace)));
    Harness {
        logger: OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
            name: callback.to_owned(),
            config: settings,
            tracer,
            emitter,
            metrics,
            tenants,
            trace: source,
        }),
        spans,
        built,
    }
}

// the tenant a routed test provider is keyed by: a request's dynamic credentials, or the key/team auth a project header comes from
#[derive(Clone, Copy)]
pub(crate) enum Tenant {
    Credentials(fn(usize) -> String),
    Project(fn(usize) -> String),
}

// one routed pre_call per index, so an earlier routed provider overflows the 256-entry cache
pub(crate) fn route_tenants(
    harness: &Harness,
    prefix: &str,
    from: usize,
    count: usize,
    tenant: Tenant,
) {
    for index in from..from.saturating_add(count) {
        let (dynamic, auth) = match tenant {
            Tenant::Credentials(space) => (Some(creds(&space(index))), None),
            Tenant::Project(project) => (
                None,
                Some(Json::Object(IndexMap::from([(
                    "phoenix_project_name".to_owned(),
                    Json::Text(project(index)),
                )]))),
            ),
        };
        pre_call(
            &harness.logger,
            PreCall {
                call_id: &format!("{prefix}{index}"),
                model: "gpt-4o",
                no_upstream: false,
                dynamic,
                auth,
            },
        );
    }
}

// one carrier-less close per index under dynamic credentials S{index}, so each close routes its own tenant and releases it
pub(crate) async fn close_credentialed_tenants(harness: &Harness, count: usize) {
    for index in 1..=count {
        success(
            &harness.logger,
            event_with(
                with_id(inference(), &format!("later-{index}")),
                creds(&format!("S{index}")),
                IndexMap::new(),
            ),
        )
        .await;
    }
}

// the logger's own provider: in memory, simple processor, so a test reads the span right after emit
fn main_tracer(
    settings: &Settings,
    factories: &ExporterFactories,
    scope: &Arc<dyn RequestScopeSource>,
    spans: &InMemorySpanExporter,
) -> SdkTracer {
    let provider = build_tracer_provider(
        settings,
        factories,
        scope,
        AssemblyOptions {
            exporter: Some(SpanSink::InMemory(spans.clone())),
            baggage_processor: None,
            use_simple_processor: Some(true),
            tenant_overrides: false,
        },
    )
    .expect("main provider");
    provider.tracer("litellm")
}

fn main_emitter(tracer: &SdkTracer) -> SpanEmitter<SdkTracer> {
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    SpanEmitter {
        tracer: tracer.clone(),
        environment,
        mappers: Vec::new(),
        event_recorder: None,
        attribute_limit: None,
        emitted: DedupCache::default(),
    }
}

pub(crate) fn with_exporters(exporters: Vec<ExporterSpec>) -> Settings {
    Settings {
        exporters,
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::SpanOnly,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

pub(crate) fn spec(
    kind: &str,
    owner: Option<ExporterOwner>,
    headers: Option<&str>,
    endpoint: Option<&str>,
) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: endpoint.map(str::to_owned),
        traces_endpoint: None,
        headers: headers.map(str::to_owned),
        owner,
        options: None,
        use_simple_processor: Some(true),
        requires_headers: false,
    }
}

pub(crate) fn owned_http() -> Settings {
    with_exporters(vec![spec(
        "otlp_http",
        Some(ExporterOwner::ArizeAx),
        None,
        None,
    )])
}

// the Phoenix preset also names its own collector endpoint, so the spec takes the URL after the shared shape
pub(crate) fn phoenix_http() -> Settings {
    let mut exporter = spec("otlp_http", Some(ExporterOwner::ArizePhoenix), None, None);
    exporter.endpoint = Some("http://phoenix:6006".to_owned());
    with_exporters(vec![exporter])
}

pub(crate) fn payload_start() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_hours(473_352))
        .expect("payload start")
}

pub(crate) fn payload_end() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_millis(1_704_067_201_500))
        .expect("payload end")
}

pub(crate) fn finished(spans: &InMemorySpanExporter) -> Vec<SpanData> {
    spans.get_finished_spans().expect("finished spans")
}

pub(crate) fn names(spans: &[SpanData]) -> Vec<String> {
    spans.iter().map(|span| span.name.to_string()).collect()
}

pub(crate) fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| pair.value.as_str().into_owned())
}

pub(crate) struct FilterSettings(pub(crate) Option<OtelModelValue>);

impl CallbackSettingsView for FilterSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.clone()
    }
}

pub(crate) fn recorder(
    attributes: Option<OtelModelValue>,
) -> (
    Arc<GenAiMetricRecorder>,
    SdkMeterProvider,
    InMemoryMetricExporter,
) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&provider.meter("otel-lifecycle-test")),
        // the global otel callback is the one whose otel.attributes filter the recorder reads
        None,
        Arc::new(FilterSettings(attributes)),
        FixedOffset::east_opt(0).expect("utc offset"),
    )
    .expect("metric recorder");
    (Arc::new(recorder), provider, exporter)
}

pub(crate) fn metric_points(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
) -> usize {
    provider.force_flush().expect("metric flush");
    exporter
        .get_finished_metrics()
        .expect("finished metrics")
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .map(|metric| match metric.data() {
            AggregatedMetrics::F64(MetricData::Histogram(histogram)) => {
                histogram.data_points().count()
            }
            AggregatedMetrics::F64(_) | AggregatedMetrics::U64(_) | AggregatedMetrics::I64(_) => 0,
        })
        .sum()
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0.lock().expect("captured messages").clone()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

fn message_of(event: &Event<'_>) -> String {
    let mut message = Message(String::new());
    event.record(&mut message);
    message.0
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let message = message_of(event);
        self.0.lock().expect("captured messages").push(message);
    }
}

// the once-only routing warnings: one message per emitted WARN record
#[derive(Clone, Default)]
pub(crate) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0.lock().expect("captured warnings").clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let message = message_of(event);
        self.0.lock().expect("captured warnings").push(message);
    }
}
