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
use crate::otel::config::{CaptureMessageContent, ExporterSpec, Settings};
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::emitter::span_emission::MapperKind;
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::logger::v2::RequestTraceSource;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::error::LoggerRuntimeError;
use crate::otel::logger_runtime::hooks::RuntimeHooks;
use crate::otel::logger_runtime::types::{
    BuiltLogger, InjectedProviders, LoggerBuildRequest, LoggerClass, LoggerRuntimeParts,
    V2LoggerConstructor,
};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::utils::tool_attr_budget;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use crate::otel::plumbing::providers::ProviderState;
use crate::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use crate::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use crate::otel::v2_callback::registry::V2Registry;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

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

#[derive(Debug)]
pub(crate) struct FixedScope;

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
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
}

#[derive(Default)]
pub(crate) struct RecordingConstructor {
    records: Mutex<Vec<BuildRecord>>,
    sinks: Mutex<Vec<InMemorySpanExporter>>,
    failure: Mutex<Option<String>>,
}

impl RecordingConstructor {
    pub(crate) fn records(&self) -> MutexGuard<'_, Vec<BuildRecord>> {
        lock(&self.records)
    }

    pub(crate) fn fail_with(&self, message: &str) {
        *lock(&self.failure) = Some(message.to_owned());
    }

    // which build's exporter a provider writes to, so a test can tell two providers apart
    pub(crate) fn sink_index(&self, provider: &SdkTracerProvider) -> Option<usize> {
        let before = self.counts();
        probe(provider);
        let after = self.counts();
        after
            .iter()
            .zip(before.iter())
            .position(|(now, was)| now > was)
    }

    fn counts(&self) -> Vec<usize> {
        lock(&self.sinks).iter().map(finished).collect()
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
        });
        let failure = lock(&self.failure).clone();
        if let Some(message) = failure {
            return Err(LoggerRuntimeError::Constructor { message });
        }
        let sink = InMemorySpanExporter::default();
        let provider = provider_exporting(&sink);
        lock(&self.sinks).push(sink);
        Ok(BuiltLogger {
            callback: Arc::new(FakeLogger { name }),
            provider,
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

pub(crate) fn fixture_with_hooks(hooks: Option<Arc<dyn RuntimeHooks>>) -> Fixture {
    build_fixture(None, true, hooks)
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

fn build_fixture_on(
    base: Settings,
    configured_exclusions: Option<Vec<String>>,
    v2_enabled: bool,
    hooks: Option<Arc<dyn RuntimeHooks>>,
) -> Fixture {
    let sdk_sink = InMemorySpanExporter::default();
    let sdk_provider = provider_exporting(&sdk_sink);
    let constructor = Arc::new(RecordingConstructor::default());
    let registered = Arc::new(V2Registry::default());
    let trace = Arc::new(FixedTrace::default());
    let environment = Arc::new(TestEnvironment::default());
    let assembler: Arc<dyn StreamOutputAssembler> = Arc::new(NullAssembler);
    let constructor_port: Arc<dyn V2LoggerConstructor> = Arc::clone(&constructor);
    let trace_source: Arc<dyn RequestTraceSource> = Arc::clone(&trace);
    let environment_lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::clone(&environment);
    let runtime = LoggerRuntime::new(LoggerRuntimeParts {
        registered: Arc::clone(&registered),
        constructor: constructor_port,
        factories: factories(),
        scope: Arc::new(FixedScope),
        trace: trace_source,
        environment: environment_lookup,
        assembler: Arc::clone(&assembler),
        base,
        v2_enabled,
        configured_exclusions,
        sdk_provider,
        hooks,
    });
    Fixture {
        runtime,
        constructor,
        registered,
        sdk_sink,
        trace,
        environment,
        assembler,
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
    OtelDestination::from(DestinationFields {
        endpoint: "https://collector.example/v1/traces".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some(callback_name.to_owned()),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}

pub(crate) fn provider_exporting(sink: &InMemorySpanExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(sink.clone()))
        .build()
}

pub(crate) fn probe(provider: &SdkTracerProvider) {
    let span = provider.tracer("logger_runtime_probe").start("probe");
    span.end();
}

pub(crate) fn finished(sink: &InMemorySpanExporter) -> usize {
    sink.get_finished_spans().map_or(0, |spans| spans.len())
}

pub(crate) fn factories() -> Arc<ExporterFactories> {
    Arc::new(ExporterFactories::new(clients(), BTreeMap::new()))
}

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

struct NoSettings;

impl CallbackSettingsView for NoSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

// B3853's callback registered the way the proxy's first V2 construction does
pub(crate) fn register_callback(
    registry: &Arc<V2Registry>,
    config: Settings,
    name: &str,
    sink: &InMemorySpanExporter,
) -> Arc<V2Callback> {
    let parts = callback_parts(config, name, sink);
    V2Callback::new(parts, registry).expect("registered callback")
}

fn callback_parts(config: Settings, name: &str, sink: &InMemorySpanExporter) -> CallbackParts {
    let factories = factories();
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let tenant_cache = tenants(&config, name, &factories, &scope);
    CallbackParts {
        callback_name: Some(name.to_owned()),
        config,
        raw_config: otel_settings::Settings::default(),
        tenants: tenant_cache,
        trace: Arc::new(FixedTrace::default()),
        providers: provider_sources(provider_exporting(sink)),
        build: ProviderBuild {
            factories,
            scope_source: scope,
            clients: clients(),
        },
        metric: MetricSettings {
            callback_settings: Arc::new(NoSettings),
            utc_offset: FixedOffset::east_opt(0).expect("utc"),
        },
        emitter: EmitterParts {
            mappers: vec![MapperKind::GenAI(GenAIMapper {
                tool_attr_budget: tool_attr_budget(1),
            })],
            environment: Arc::new(BTreeMap::<String, String>::new()),
            attribute_limit: None,
        },
        error_info: ErrorInformationConfig {
            normalizer: Normalizer::new().expect("normalizer"),
            redactor: None,
            log_client_error_tracebacks: false,
        },
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
