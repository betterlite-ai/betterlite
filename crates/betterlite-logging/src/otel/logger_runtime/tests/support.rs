#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use chrono::FixedOffset;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor, SpanData,
};

use crate::callback_logger::CallbackLogger;
use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::emitter::span_emission::MapperKind;
use crate::otel::fan_out::attach::TenantFanOutRegistry;
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::langfuse_root::{RootSpanSink, RootSpanSource};
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::logger::v2::RequestTraceSource;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::construction::CallbackConstructor;
use crate::otel::logger_runtime::error::LoggerRuntimeError;
use crate::otel::logger_runtime::hooks::RuntimeHooks;
use crate::otel::logger_runtime::types::{
    BuiltLogger, ConstructorSettings, InjectedProviders, LoggerBuildRequest, LoggerClass,
    V2LoggerConstructor,
};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::utils::tool_attr_budget;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, HttpClients,
};
use crate::otel::plumbing::providers::ProviderState;
use crate::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use crate::otel::plumbing::sinks::SpanSink;
use crate::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use crate::otel::v2_callback::error::CallbackError;
use crate::otel::v2_callback::registry::V2Registry;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_fan_out_tracer_provider,
};

pub(crate) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    match value.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn attribute<'data>(data: &'data SpanData, key: &str) -> Option<&'data Value> {
    data.attributes
        .iter()
        .find(|entry| entry.key.as_str() == key)
        .map(|entry| &entry.value)
}

#[derive(Debug)]
struct NoClient;

#[async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the logger runtime tests export in memory"))
    }
}

#[derive(Debug, Default)]
pub(crate) struct FixedScope {
    scope: Mutex<RequestOtelScope>,
}

impl FixedScope {
    pub(crate) fn set_destinations(&self, destinations: Vec<OtelDestination>) {
        lock(&self.scope).destinations = destinations;
    }
}

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        lock(&self.scope).clone()
    }
}

#[derive(Debug, Default)]
pub(crate) struct SinkRecorder {
    sinks: Mutex<Vec<InMemorySpanExporter>>,
}

impl SinkRecorder {
    fn exporter(&self) -> InMemorySpanExporter {
        let sink = InMemorySpanExporter::default();
        lock(&self.sinks).push(sink.clone());
        sink
    }

    pub(crate) fn finished(&self) -> usize {
        lock(&self.sinks).iter().map(finished).sum()
    }

    pub(crate) fn spans(&self) -> Vec<SpanData> {
        lock(&self.sinks)
            .iter()
            .flat_map(|sink| sink.get_finished_spans().unwrap_or_default())
            .collect()
    }
}

#[derive(Debug, Default)]
pub(crate) struct FixedTrace {
    context: Mutex<RequestTraceContext>,
}

impl FixedTrace {
    pub(crate) fn set_destinations(&self, destinations: Vec<OtelDestination>) {
        lock(&self.context).destinations = destinations;
    }
}

impl RequestTraceSource for FixedTrace {
    fn trace_context(&self) -> RequestTraceContext {
        lock(&self.context).clone()
    }
}

#[derive(Debug, Default)]
pub(crate) struct TestEnvironment {
    values: Mutex<BTreeMap<String, String>>,
}

impl TestEnvironment {
    pub(crate) fn set(&self, name: &str, value: &str) {
        lock(&self.values).insert(name.to_owned(), value.to_owned());
    }
}

impl EnvironmentLookup for TestEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        lock(&self.values).get(name).cloned()
    }
}

pub(crate) struct NullAssembler;

impl StreamOutputAssembler for NullAssembler {
    fn assemble_sse(&self, _frames: &[Vec<u8>]) -> Option<OtelModelValue> {
        None
    }

    fn assemble_chat(
        &self,
        _chunks: &[OtelModelValue],
        _messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue> {
        None
    }
}

pub(crate) struct FakeLogger {
    pub(crate) name: String,
}

#[async_trait]
impl CallbackLogger for FakeLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

pub(crate) struct BuildRecord {
    pub(crate) class: LoggerClass,
    pub(crate) callback_name: Option<String>,
    pub(crate) providers: Vec<&'static str>,
    pub(crate) assembler: Option<Arc<dyn StreamOutputAssembler>>,
    pub(crate) config: Settings,
    pub(crate) settings: ConstructorSettings,
}

#[derive(Default)]
pub(crate) struct RecordingConstructor {
    records: Mutex<Vec<BuildRecord>>,
    failure: Mutex<Option<CallbackError>>,
}

impl RecordingConstructor {
    pub(crate) fn records(&self) -> MutexGuard<'_, Vec<BuildRecord>> {
        lock(&self.records)
    }

    pub(crate) fn fail_with(&self, error: CallbackError) {
        *lock(&self.failure) = Some(error);
    }
}

impl V2LoggerConstructor for RecordingConstructor {
    fn build(&self, request: &LoggerBuildRequest<'_>) -> Result<BuiltLogger, LoggerRuntimeError> {
        let name = request
            .callback_name
            .map_or_else(|| "generic".to_owned(), str::to_owned);
        lock(&self.records).push(BuildRecord {
            class: request.class,
            callback_name: request.callback_name.map(str::to_owned),
            providers: injected_providers(request.providers),
            assembler: request.assembler.cloned(),
            config: request.config.clone(),
            settings: request.settings,
        });
        let failure = lock(&self.failure).take();
        if let Some(source) = failure {
            return Err(LoggerRuntimeError::Constructor(source));
        }
        let provider = request
            .providers
            .tracer_provider
            .cloned()
            .expect("the factory injects a provider");
        Ok(BuiltLogger {
            callback: Arc::new(FakeLogger { name }),
            hooks: None,
            provider,
            recorder_providers: (None, None),
        })
    }
}

fn injected_providers(providers: InjectedProviders<'_>) -> Vec<&'static str> {
    let mut names = Vec::new();
    if providers.tracer_provider.is_some() {
        names.push("tracer");
    }
    if providers.meter_provider.is_some() {
        names.push("meter");
    }
    if providers.logger_provider.is_some() {
        names.push("logger");
    }
    names
}

pub(crate) struct Fixture {
    pub(crate) runtime: LoggerRuntime,
    pub(crate) constructor: Arc<RecordingConstructor>,
    pub(crate) registered: Arc<V2Registry>,
    pub(crate) sdk_sink: InMemorySpanExporter,
    pub(crate) trace: Arc<FixedTrace>,
    pub(crate) environment: Arc<TestEnvironment>,
    pub(crate) assembler: Arc<dyn StreamOutputAssembler>,
    pub(crate) scope: Arc<FixedScope>,
    pub(crate) export_sinks: Arc<SinkRecorder>,
    pub(crate) tenant_sinks: Arc<SinkRecorder>,
}

pub(crate) fn fixture() -> Fixture {
    build_fixture(None, true, None)
}

pub(crate) fn fixture_excluding(configured: Option<Vec<String>>) -> Fixture {
    build_fixture(configured, true, None)
}

pub(crate) fn fixture_disabled() -> Fixture {
    build_fixture(None, false, None)
}

pub(crate) fn fixture_with_base(base: Settings) -> Fixture {
    build_fixture_on(base, None, true, None)
}

fn build_fixture(
    configured_exclusions: Option<Vec<String>>,
    v2_enabled: bool,
    hooks: Option<Arc<dyn RuntimeHooks>>,
) -> Fixture {
    build_fixture_on(
        settings(vec![placeholder()]),
        configured_exclusions,
        v2_enabled,
        hooks,
    )
}

struct FixtureParts {
    sdk_sink: InMemorySpanExporter,
    sdk_provider: SdkTracerProvider,
    constructor: Arc<RecordingConstructor>,
    registered: Arc<V2Registry>,
    trace: Arc<FixedTrace>,
    environment: Arc<TestEnvironment>,
    assembler: Arc<dyn StreamOutputAssembler>,
    scope: Arc<FixedScope>,
    factories: Arc<ExporterFactories>,
    export_sinks: Arc<SinkRecorder>,
    tenant_sinks: Arc<SinkRecorder>,
}

fn fixture_parts() -> FixtureParts {
    let sdk_sink = InMemorySpanExporter::default();
    let sdk_provider = provider_exporting(&sdk_sink);
    let (factories, export_sinks, tenant_sinks) = fixture_factories();
    FixtureParts {
        sdk_sink,
        sdk_provider,
        constructor: Arc::new(RecordingConstructor::default()),
        registered: Arc::new(V2Registry::default()),
        trace: Arc::new(FixedTrace::default()),
        environment: Arc::new(TestEnvironment::default()),
        assembler: Arc::new(NullAssembler),
        scope: Arc::new(FixedScope::default()),
        factories,
        export_sinks,
        tenant_sinks,
    }
}

fn build_fixture_on(
    base: Settings,
    configured_exclusions: Option<Vec<String>>,
    v2_enabled: bool,
    hooks: Option<Arc<dyn RuntimeHooks>>,
) -> Fixture {
    let parts = fixture_parts();
    let runtime = LoggerRuntime {
        registered: Arc::clone(&parts.registered),
        loggers: Mutex::new(Vec::new()),
        fan_out: Arc::new(Mutex::new(TenantFanOutRegistry::default())),
        constructor: Arc::clone(&parts.constructor),
        base,
        environment: Arc::clone(&parts.environment),
        trace: Arc::clone(&parts.trace),
        factories: Arc::clone(&parts.factories),
        scope: Arc::clone(&parts.scope),
        assembler: Arc::clone(&parts.assembler),
        hooks,
        configured_exclusions,
        v2_enabled,
        sdk_provider: parts.sdk_provider,
        generic: Mutex::new(None),
        published: Mutex::new(None),
    };
    Fixture {
        runtime,
        constructor: parts.constructor,
        registered: parts.registered,
        sdk_sink: parts.sdk_sink,
        trace: parts.trace,
        environment: parts.environment,
        assembler: parts.assembler,
        scope: parts.scope,
        export_sinks: parts.export_sinks,
        tenant_sinks: parts.tenant_sinks,
    }
}

pub(crate) fn settings(exporters: Vec<ExporterSpec>) -> Settings {
    Settings {
        exporters,
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned()],
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

pub(crate) fn spec(
    kind: &str,
    endpoint: Option<&str>,
    headers: Option<&str>,
    requires_headers: bool,
) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: endpoint.map(str::to_owned),
        traces_endpoint: None,
        headers: headers.map(str::to_owned),
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers,
    }
}

pub(crate) fn placeholder() -> ExporterSpec {
    spec("console", None, None, false)
}

pub(crate) fn fixture_sink() -> ExporterSpec {
    spec("fixture_sink", None, None, false)
}

pub(crate) fn owned_fixture_sink(owner: ExporterOwner) -> ExporterSpec {
    ExporterSpec {
        owner: Some(owner),
        ..fixture_sink()
    }
}

pub(crate) fn gated() -> ExporterSpec {
    spec("console", None, None, true)
}

pub(crate) fn operator_exporter() -> ExporterSpec {
    spec(
        "otlp_http",
        Some("https://collector.example/v1/traces"),
        Some("authorization=Bearer operator"),
        false,
    )
}

pub(crate) fn destination(callback_name: &str) -> OtelDestination {
    destination_with_protocol(callback_name, None)
}

pub(crate) fn tenant_destination(callback_name: &str) -> OtelDestination {
    destination_with_protocol(callback_name, Some("tenant_sink"))
}

fn destination_with_protocol(callback_name: &str, protocol: Option<&str>) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: "https://collector.example/v1/traces".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some(callback_name.to_owned()),
        protocol: protocol.map(str::to_owned),
        span_scope: SpanScope::Full,
    })
}

pub(crate) fn provider_exporting(sink: &InMemorySpanExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(sink.clone()))
        .build()
}

pub(crate) fn probe(provider: &SdkTracerProvider) {
    let mut span = provider.tracer("logger_runtime_probe").start("probe");
    span.end();
}

pub(crate) fn finished(sink: &InMemorySpanExporter) -> usize {
    sink.get_finished_spans().map_or(0, |spans| spans.len())
}

fn fixture_factories() -> (Arc<ExporterFactories>, Arc<SinkRecorder>, Arc<SinkRecorder>) {
    let export_sinks = Arc::new(SinkRecorder::default());
    let tenant_sinks = Arc::new(SinkRecorder::default());
    let custom = BTreeMap::from([
        ("fixture_sink".to_owned(), custom_factory(&export_sinks)),
        ("tenant_sink".to_owned(), custom_factory(&tenant_sinks)),
    ]);
    (
        Arc::new(ExporterFactories::new(clients(), custom)),
        export_sinks,
        tenant_sinks,
    )
}

fn custom_factory(recorder: &Arc<SinkRecorder>) -> ExporterFactory {
    let sinks = Arc::clone(recorder);
    let factory = move |_spec: &ExporterSpec| -> ExportResult<SpanSink> {
        Ok(SpanSink::InMemory(sinks.exporter()))
    };
    Arc::new(factory)
}

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

pub(crate) struct NoSettings;

impl CallbackSettingsView for NoSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

// the production constructor, carrying the caller's root span source
pub(crate) fn callback_constructor(roots: Arc<dyn RootSpanSource>) -> CallbackConstructor {
    CallbackConstructor {
        raw_config: otel_settings::Settings::default(),
        callback_settings: Arc::new(NoSettings),
        allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        roots,
        attribute_limit: None,
        redactor: None,
        log_client_error_tracebacks: false,
        utc_offset: FixedOffset::east_opt(0).expect("utc"),
    }
}

pub(crate) type RootLog = Arc<Mutex<Vec<String>>>;

#[derive(Debug)]
pub(crate) struct RecordingSink {
    recording: bool,
    log: RootLog,
}

impl RootSpanSink for RecordingSink {
    fn is_recording(&self) -> bool {
        self.recording
    }

    fn set_attribute(&self, key: &str, value: String) {
        lock(&self.log).push(format!("{key}={value}"));
    }
}

pub(crate) struct FixedRoots {
    pub(crate) sink: Option<Arc<dyn RootSpanSink>>,
}

impl RootSpanSource for FixedRoots {
    fn request_root_span(&self) -> Option<Arc<dyn RootSpanSink>> {
        self.sink.clone()
    }
}

// the recording root span the Langfuse pre-call and content hooks stamp
pub(crate) fn recording_root() -> (RootLog, Arc<dyn RootSpanSource>) {
    let log: RootLog = Arc::default();
    let sink: Arc<dyn RootSpanSink> = Arc::new(RecordingSink {
        recording: true,
        log: Arc::clone(&log),
    });
    let roots: Arc<dyn RootSpanSource> = Arc::new(FixedRoots { sink: Some(sink) });
    (log, roots)
}

// B3853's callback registered the way the proxy's first V2 construction does
pub(crate) fn register_callback(
    fixture: &Fixture,
    config: Settings,
    name: &str,
) -> Arc<V2Callback> {
    let parts = callback_parts(fixture, config, name);
    V2Callback::new(parts, &fixture.registered).expect("registered callback")
}

fn callback_parts(fixture: &Fixture, config: Settings, name: &str) -> CallbackParts {
    let factories = Arc::clone(&fixture.runtime.factories);
    let scope = Arc::clone(&fixture.runtime.scope);
    let tenant_cache = tenants(&config, name, &factories, &scope);
    let provider = build_fan_out_tracer_provider(
        &config,
        Arc::clone(&factories),
        &scope,
        AssemblyOptions {
            exporter: Some(SpanSink::InMemory(fixture.export_sinks.exporter())),
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides: true,
        },
        config.excluded_services.clone(),
    )
    .expect("the registered provider builds");
    CallbackParts {
        callback_name: Some(name.to_owned()),
        config,
        raw_config: otel_settings::Settings::default(),
        tenants: tenant_cache,
        trace: Arc::new(FixedTrace::default()),
        providers: provider_sources(provider),
        build: ProviderBuild {
            factories,
            scope_source: scope,
            clients: clients(),
        },
        metric: MetricSettings {
            callback_settings: Arc::new(NoSettings),
            utc_offset: FixedOffset::east_opt(0).expect("utc"),
        },
        emitter: emitter_parts(),
        error_info: error_information(),
    }
}

fn emitter_parts() -> EmitterParts {
    EmitterParts {
        mappers: vec![MapperKind::GenAI(GenAIMapper {
            tool_attr_budget: tool_attr_budget(1),
        })],
        environment: Arc::new(BTreeMap::<String, String>::new()),
        attribute_limit: None,
    }
}

fn error_information() -> ErrorInformationConfig {
    ErrorInformationConfig {
        normalizer: Normalizer::new().expect("normalizer"),
        redactor: None,
        log_client_error_tracebacks: false,
    }
}

fn tenants(
    config: &Settings,
    name: &str,
    factories: &Arc<ExporterFactories>,
    scope: &Arc<dyn RequestScopeSource>,
) -> TenantTracerCache {
    TenantTracerCache::new(
        config.clone(),
        Some(name.to_owned()),
        "litellm",
        TenantSources {
            factories: Arc::clone(factories),
            scope_source: Arc::clone(scope),
            allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        },
    )
}

fn provider_sources(provider: SdkTracerProvider) -> ProviderSources {
    ProviderSources {
        tracer_provider: Some(provider),
        meter_provider: None,
        logger_provider: None,
        meter_state: ProviderState::Unset,
        logger_state: ProviderState::Unset,
        metrics: None,
        events: None,
    }
}
