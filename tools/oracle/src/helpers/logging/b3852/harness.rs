use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_pre_post::hooks::PreCallEvent;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{
    CaptureMessageContent, ExporterOwner, ExporterSpec, OTelConfigError, Settings,
};
use betterlite_logging::otel::emitter::span_emission::{DedupCache, SpanEmitter};
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::pre_call::log_pre_api_call;
use betterlite_logging::otel::logger::v2::{
    OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource,
};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, RequestTraceContext,
};
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use betterlite_logging::otel::plumbing::routing::cache::TenantTracerCache;
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use betterlite_logging::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use chrono::FixedOffset;
use indexmap::IndexMap;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceFlags, TraceId, TraceState, TracerProvider as _,
};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SpanData};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer};

pub(super) const MODEL: &str = "gpt-4o";
pub(super) const CALL_ID: &str = "call-1";

// the custom exporter kind both sides register, so a routed provider builds an in-memory sink
const KIND: &str = "b3852_memory";

// the same anchored request root the pin's OTel tests use
const ROOT_TRACE: u128 = 0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c;
const ROOT_SPAN: u64 = 0xb7ad_6b71_6920_3331;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "litellm_call_id": "call-1",
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

#[derive(Debug, thiserror::Error)]
pub(super) enum HarnessError {
    #[error("the payload fixture is not an object")]
    Fixture,
    #[error("no utc offset")]
    Offset,
    #[error("the route built no provider to inspect")]
    NoProvider,
    #[error("the eviction scenario needs at least one space")]
    NoSpaces,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExporterInput {
    kind: String,
    endpoint: Option<String>,
    headers: Option<String>,
    owner: Option<String>,
    use_simple_processor: bool,
}

pub(super) struct Harness {
    pub(super) logger: OpenTelemetryV2,
    pub(super) default: InMemorySpanExporter,
    pub(super) built: Built,
}

pub(super) struct CacheHarness {
    pub(super) cache: TenantTracerCache,
    pub(super) default: SdkTracer,
    pub(super) built: Built,
}

#[derive(Clone, Default)]
pub(super) struct Built(Arc<BuiltInner>);

#[derive(Default)]
struct BuiltInner {
    specs: Mutex<Vec<ExporterSpec>>,
    sinks: Mutex<Vec<InMemorySpanExporter>>,
}

impl Built {
    fn record(&self, spec: &ExporterSpec, sink: &InMemorySpanExporter) {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(spec.clone());
        self.0
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sink.clone());
    }

    pub(super) fn builds(&self) -> usize {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn specs(&self) -> Vec<ExporterSpec> {
        self.0
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sinks(&self) -> Vec<InMemorySpanExporter> {
        self.0
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// counts the ERROR records the once-only metric filter log emits
#[derive(Clone, Default)]
pub(super) struct Captured(Arc<AtomicUsize>);

impl Captured {
    pub(super) fn count(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn settings(exporters: &[ExporterInput]) -> Result<Settings, Box<HelperError>> {
    Ok(Settings {
        exporters: exporters
            .iter()
            .map(exporter_spec)
            .collect::<Result<Vec<_>, _>>()?,
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    })
}

fn exporter_spec(input: &ExporterInput) -> Result<ExporterSpec, Box<HelperError>> {
    Ok(ExporterSpec {
        kind: input.kind.clone(),
        endpoint: input.endpoint.clone(),
        traces_endpoint: None,
        headers: input.headers.clone(),
        owner: input.owner.as_deref().map(exporter_owner).transpose()?,
        options: None,
        use_simple_processor: Some(input.use_simple_processor),
        requires_headers: false,
    })
}

fn exporter_owner(name: &str) -> Result<ExporterOwner, Box<HelperError>> {
    let owners = [
        ("arize", ExporterOwner::ArizeAx),
        ("arize_phoenix", ExporterOwner::ArizePhoenix),
        ("langfuse_otel", ExporterOwner::LangfuseOtel),
        ("weave_otel", ExporterOwner::WeaveOtel),
        ("levo", ExporterOwner::Levo),
        ("agentops", ExporterOwner::Agentops),
        ("newrelic", ExporterOwner::Newrelic),
        ("signoz", ExporterOwner::Signoz),
    ];
    owners
        .iter()
        .find_map(|(candidate, owner)| (*candidate == name).then_some(*owner))
        .ok_or_else(|| OTelConfigError::ExporterOwner(name.to_owned()))
        .or_config_error()
}

// the request's own start stands in for an absent carrier at close, which is exactly what the
// anchored request context carries when pre_call saw a recordable parent
pub(super) fn trace(anchored: bool) -> RequestTraceContext {
    let root = anchored.then(|| AnchoredSpan {
        context: SpanContext::new(
            TraceId::from(ROOT_TRACE),
            SpanId::from(ROOT_SPAN),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ),
        http_route: Some("/v1/chat/completions".to_owned()),
    });
    RequestTraceContext {
        root,
        ..RequestTraceContext::default()
    }
}

pub(super) fn harness(
    settings: Settings,
    callback: Option<String>,
    metrics: Option<Arc<GenAiMetricRecorder>>,
    trace: RequestTraceContext,
) -> Result<Harness, Box<HelperError>> {
    let built = Built::default();
    let default = InMemorySpanExporter::default();
    let factories = factories(&built);
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let sink = SpanSink::InMemory(default.clone());
    let tracer = default_tracer(&settings, &factories, &scope, sink)?;
    let emitter = span_emitter(&tracer);
    let tenants = TenantTracerCache::new(
        settings.clone(),
        callback.clone(),
        "litellm",
        factories,
        Arc::clone(&scope),
        hosts(),
    );
    Ok(Harness {
        logger: OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
            name: callback.unwrap_or_default(),
            config: settings,
            tracer,
            emitter,
            metrics,
            tenants,
            trace: Arc::new(FixedTrace(Mutex::new(trace))),
        }),
        default,
        built,
    })
}

pub(super) fn cache(
    settings: Settings,
    callback: Option<String>,
) -> Result<CacheHarness, Box<HelperError>> {
    let built = Built::default();
    let factories = factories(&built);
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let sink = SpanSink::InMemory(InMemorySpanExporter::default());
    let default = default_tracer(&settings, &factories, &scope, sink)?;
    Ok(CacheHarness {
        cache: TenantTracerCache::new(settings, callback, "litellm", factories, scope, hosts()),
        default,
        built,
    })
}

// the provider the logger emits on when no tenant routes the request, in memory so nothing leaves
fn default_tracer(
    settings: &Settings,
    factories: &ExporterFactories,
    scope: &Arc<dyn RequestScopeSource>,
    sink: SpanSink,
) -> Result<SdkTracer, Box<HelperError>> {
    let provider = build_tracer_provider(
        settings,
        factories,
        scope,
        AssemblyOptions {
            exporter: Some(sink),
            use_simple_processor: Some(true),
            ..AssemblyOptions::default()
        },
    )
    .or_config_error()?;
    Ok(provider.tracer("litellm"))
}

fn span_emitter(tracer: &SdkTracer) -> SpanEmitter<SdkTracer> {
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

fn factories(built: &Built) -> Arc<ExporterFactories> {
    let recorder = built.clone();
    let factory: ExporterFactory = Arc::new(move |spec: &ExporterSpec| -> ExportResult<SpanSink> {
        let sink = InMemorySpanExporter::default();
        recorder.record(spec, &sink);
        Ok(SpanSink::InMemory(sink))
    });
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    Arc::new(ExporterFactories::new(
        HttpClients {
            traces: Arc::clone(&client),
            metrics: Arc::clone(&client),
            logs: client,
        },
        BTreeMap::from([(KIND.to_owned(), factory)]),
    ))
}

fn hosts() -> Arc<[AllowedDestination]> {
    Arc::from(Vec::<AllowedDestination>::new())
}

#[derive(Debug)]
struct FixedScope;

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

#[derive(Debug)]
struct FixedTrace(Mutex<RequestTraceContext>);

impl RequestTraceSource for FixedTrace {
    fn trace_context(&self) -> RequestTraceContext {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug)]
struct NoClient;

#[async_trait::async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the lifecycle scenarios export in memory"))
    }
}

struct FilterSettings(Option<OtelModelValue>);

impl CallbackSettingsView for FilterSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.clone()
    }
}

pub(super) fn metric_recorder(
    attributes: OtelModelValue,
) -> Result<(Arc<GenAiMetricRecorder>, SdkMeterProvider), Box<HelperError>> {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&provider.meter("b3852")),
        // the filter is read only for the unnamed callback and for "otel", as LiteLLM reads it
        None,
        Arc::new(FilterSettings(Some(attributes))),
        FixedOffset::east_opt(0)
            .ok_or(HarnessError::Offset)
            .or_value_error()?,
    )
    .or_value_error()?;
    Ok((Arc::new(recorder), provider))
}

pub(super) fn text_object(params: &BTreeMap<String, String>) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(key, value)| (key.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

pub(super) fn dynamic_params(params: Option<&BTreeMap<String, String>>) -> Json {
    params.map_or(Json::Null, text_object)
}

pub(super) fn space_params(space: &str) -> Json {
    Json::Object(IndexMap::from([(
        "arize_space_id".to_owned(),
        Json::Text(space.to_owned()),
    )]))
}

pub(super) fn pre_call_details(
    credentials: Option<&BTreeMap<String, String>>,
    auth_metadata: Option<&BTreeMap<String, String>>,
) -> Json {
    let mut details = IndexMap::from([
        ("model".to_owned(), Json::Text(MODEL.to_owned())),
        ("call_type".to_owned(), Json::Text("completion".to_owned())),
        ("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned())),
    ]);
    if let Some(credentials) = credentials {
        details.insert(
            "standard_callback_dynamic_params".to_owned(),
            text_object(credentials),
        );
    }
    if let Some(auth_metadata) = auth_metadata {
        details.insert(
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([(
                "metadata".to_owned(),
                Json::Object(IndexMap::from([(
                    "user_api_key_auth_metadata".to_owned(),
                    text_object(auth_metadata),
                )])),
            )])),
        );
    }
    Json::Object(details)
}

// litellm.integrations.otel.logger.OpenTelemetryV2.log_pre_api_call, as the proxy calls it
pub(super) fn boundary(
    logger: &OpenTelemetryV2,
    credentials: Option<&BTreeMap<String, String>>,
    auth_metadata: Option<&BTreeMap<String, String>>,
) -> Result<(), Box<HelperError>> {
    let details = pre_call_details(credentials, auth_metadata);
    let messages = Json::List(Vec::new());
    log_pre_api_call(
        logger,
        &PreCallEvent {
            model: MODEL,
            messages: &messages,
            details: &details,
        },
    )
    .or_value_error()
}

pub(super) fn payload_metadata(
    auth_metadata: Option<&BTreeMap<String, String>>,
    tool_name: Option<&str>,
) -> Value {
    let mut metadata = serde_json::Map::new();
    if let Some(tool_name) = tool_name {
        metadata.insert(
            "mcp_tool_call_metadata".to_owned(),
            json!({"name": tool_name}),
        );
    }
    if let Some(auth_metadata) = auth_metadata {
        metadata.insert(
            "user_api_key_auth_metadata".to_owned(),
            json!(auth_metadata),
        );
    }
    Value::Object(metadata)
}

pub(super) fn payload_document(
    call_type: &str,
    status: &str,
    metadata: Value,
) -> Result<StandardLoggingPayload, Box<HelperError>> {
    let mut value: Value = serde_json::from_str(PAYLOAD)?;
    let object = value
        .as_object_mut()
        .ok_or(HarnessError::Fixture)
        .or_value_error()?;
    object.insert("call_type".to_owned(), Value::String(call_type.to_owned()));
    object.insert("status".to_owned(), Value::String(status.to_owned()));
    if status == "failure" {
        object.insert(
            "status_fields".to_owned(),
            json!({"llm_api_status": "failure", "guardrail_status": "not_run"}),
        );
    }
    object.insert("metadata".to_owned(), metadata);
    Ok(serde_json::from_value(value)?)
}

pub(super) fn event(payload: StandardLoggingPayload, dynamic: Json) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn deliver(
    logger: &OpenTelemetryV2,
    event: Arc<LoggerEvent>,
    failure: bool,
) -> Result<(), Box<HelperError>> {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    runtime.block_on(async {
        if failure {
            logger.async_log_failure_event(event).await
        } else {
            logger.async_log_success_event(event).await
        }
        .or_value_error()
    })
}

fn finished(sinks: &[InMemorySpanExporter]) -> Result<Vec<SpanData>, Box<HelperError>> {
    let mut spans = Vec::new();
    for sink in sinks {
        spans.extend(sink.get_finished_spans().or_value_error()?);
    }
    Ok(spans)
}

pub(super) fn span_records(sinks: &[InMemorySpanExporter]) -> Result<Vec<Json>, Box<HelperError>> {
    let mut records = Vec::new();
    for span in finished(sinks)? {
        records.push(json!({
            "name": span.name,
            "rooted": span.parent_span_id == SpanId::INVALID,
            "links": span.links.links.len(),
        }));
    }
    Ok(records)
}

pub(super) fn span_names(sinks: &[InMemorySpanExporter]) -> Result<Vec<String>, Box<HelperError>> {
    let mut names = Vec::new();
    for span in finished(sinks)? {
        names.push(span.name.to_string());
    }
    names.sort();
    Ok(names)
}

pub(super) fn shutdowns(sinks: &[InMemorySpanExporter]) -> usize {
    let mut count = 0;
    for sink in sinks {
        if sink.is_shutdown_called() {
            count = count.saturating_add(1);
        }
    }
    count
}
