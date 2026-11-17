#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{
    CaptureMessageContent, ExporterOwner, ExporterSpec, Settings,
};
use betterlite_logging::otel::emitter::span_emission::MapperKind;
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::v2::RequestTraceSource;
use betterlite_logging::otel::mappers::genai::GenAIMapper;
use betterlite_logging::otel::mappers::utils::tool_attr_budget;
use betterlite_logging::otel::model::payloads::error::SpanError;
use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::context::post_response::ServiceSpanTiming;
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::providers::ProviderState;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::service::ServiceHook;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use chrono::FixedOffset;
use opentelemetry::trace::{SpanContext, Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracer, SdkTracerProvider, SimpleSpanProcessor, SpanData,
};
use serde_json::Number;

pub(crate) const CALLBACK: &str = "b3853";

pub(crate) fn settings() -> Settings {
    Settings {
        exporters: Vec::new(),
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::SpanOnly,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned()],
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: vec!["litellm.team.id".to_owned()],
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

pub(crate) fn exporter_spec(kind: &str) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        owner: Some(ExporterOwner::ArizeAx),
        options: None,
        use_simple_processor: Some(true),
        requires_headers: false,
    }
}

// an exporter kind that hands the provider an in-memory sink the test reads after the span ends
pub(crate) fn memory_factories(sink: &InMemorySpanExporter) -> Arc<ExporterFactories> {
    let exporter = sink.clone();
    let factory: ExporterFactory =
        Arc::new(move |_spec: &ExporterSpec| -> ExportResult<SpanSink> {
            Ok(SpanSink::InMemory(exporter.clone()))
        });
    Arc::new(ExporterFactories::new(
        clients(),
        BTreeMap::from([("b3853_memory".to_owned(), factory)]),
    ))
}

pub(crate) fn construction_parts(
    settings: Settings,
    spans: &InMemorySpanExporter,
    provider: Option<SdkTracerProvider>,
) -> CallbackParts {
    CallbackParts {
        callback_name: Some(CALLBACK.to_owned()),
        config: settings.clone(),
        raw_config: otel_settings::Settings::default(),
        tenants: tenants(&settings, spans),
        trace: Arc::new(FixedTrace::default()),
        providers: ProviderSources {
            tracer_provider: provider,
            meter_provider: None,
            logger_provider: None,
            meter_state: ProviderState::Unset,
            logger_state: ProviderState::Unset,
            metrics: None,
            events: None,
        },
        build: ProviderBuild {
            factories: memory_factories(spans),
            scope_source: Arc::new(FixedScope),
            clients: clients(),
        },
        metric: MetricSettings {
            callback_settings: Arc::new(FilterSettings),
            utc_offset: FixedOffset::east_opt(0).expect("utc"),
        },
        emitter: EmitterParts {
            mappers: genai_mappers(),
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

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

fn tenants(settings: &Settings, spans: &InMemorySpanExporter) -> TenantTracerCache {
    let sources = TenantSources {
        factories: memory_factories(spans),
        scope_source: Arc::new(FixedScope),
        allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
    };
    TenantTracerCache::new(
        settings.clone(),
        Some(CALLBACK.to_owned()),
        "litellm",
        sources,
    )
}

fn genai_mappers() -> Vec<MapperKind> {
    vec![MapperKind::GenAI(GenAIMapper {
        tool_attr_budget: tool_attr_budget(1),
    })]
}

#[derive(Debug)]
struct NoClient;

#[async_trait::async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the v2 callback tests export in memory"))
    }
}

#[derive(Debug)]
struct FixedScope;

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

#[derive(Debug, Default)]
struct FixedTrace(Mutex<RequestTraceContext>);

impl RequestTraceSource for FixedTrace {
    fn trace_context(&self) -> RequestTraceContext {
        match self.0.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

struct FilterSettings;

impl CallbackSettingsView for FilterSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

pub(crate) struct Harness {
    pub(crate) callback: Arc<V2Callback>,
    pub(crate) registry: V2Registry,
    pub(crate) spans: InMemorySpanExporter,
    tracer: SdkTracer,
}

// a logger whose tracer and sink the test owns, built from the parts either harness hands it
fn assemble(
    settings: Settings,
    spans: &InMemorySpanExporter,
    provider: Option<SdkTracerProvider>,
    tracer: SdkTracer,
) -> Harness {
    let registry = V2Registry::default();
    let parts = construction_parts(settings, spans, provider);
    let callback = V2Callback::new(parts, &registry).expect("callback");
    Harness {
        callback,
        registry,
        spans: spans.clone(),
        tracer,
    }
}

// an injected provider, so a test owns both the tracer and the sink it exports to
pub(crate) fn harness(settings: Settings) -> Harness {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build();
    let tracer = provider.tracer("litellm");
    assemble(settings, &spans, Some(provider), tracer)
}

// no provider handed in, so construction builds one against the memory exporter
pub(crate) fn built_harness(settings: Settings) -> Harness {
    let spans = InMemorySpanExporter::default();
    let tracer = SdkTracerProvider::builder()
        .build()
        .tracer("built-elsewhere");
    assemble(settings, &spans, None, tracer)
}

impl Harness {
    pub(crate) fn span(&self, name: &str) -> LiveSpan {
        LiveSpan::from(self.tracer.start(name.to_owned()))
    }

    pub(crate) fn finished(&self) -> Vec<SpanData> {
        self.spans.get_finished_spans().expect("finished spans")
    }
}

pub(crate) fn service_payload(service: ServiceType, call_type: &str) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        service,
        call_type: call_type.to_owned(),
        caller: Some("test".to_owned()),
        error: None,
    }
}

pub(crate) fn timing(end: Option<SystemTime>) -> ServiceSpanTiming {
    ServiceSpanTiming {
        end_time: end,
        parent_end_time: None,
        post_response: false,
    }
}

pub(crate) fn hook<'hook>(
    payload: &'hook ServiceLoggerPayload,
    parent: Option<&'hook SpanContext>,
    start: Option<SystemTime>,
    timing: ServiceSpanTiming,
    event_metadata: Option<&'hook BTreeMap<String, OtelModelValue>>,
) -> ServiceHook<'hook> {
    ServiceHook {
        payload,
        parent_otel_span: parent,
        start_time: start,
        timing,
        event_metadata,
    }
}

// LiteLLM's span keeps a dict of attributes, so a repeated key carries the last value it was given
pub(crate) fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .rev()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| pair.value.to_string())
}

pub(crate) fn event_names(span: &SpanData) -> Vec<String> {
    span.events
        .iter()
        .map(|event| event.name.to_string())
        .collect()
}

pub(crate) fn guardrail_entry(name: &str) -> BTreeMap<String, OtelModelValue> {
    BTreeMap::from([
        (
            "guardrail_name".to_owned(),
            OtelModelValue::String(name.to_owned()),
        ),
        (
            "guardrail_status".to_owned(),
            OtelModelValue::String("success".to_owned()),
        ),
        (
            "start_time".to_owned(),
            OtelModelValue::Number(Number::from_f64(1_704_067_200.0).expect("start")),
        ),
        (
            "end_time".to_owned(),
            OtelModelValue::Number(Number::from_f64(1_704_067_201.5).expect("end")),
        ),
    ])
}

pub(crate) fn exception_facts(status: u16) -> ExceptionFacts {
    ExceptionFacts {
        class_name: "AuthenticationError".to_owned(),
        class: None,
        message: Some("bad credentials".to_owned()),
        code: None,
        status_code: Some(status),
        response_status_code: None,
        llm_provider: Some("openai".to_owned()),
        category: None,
        rate_limit_type: None,
        budget_entity_type: None,
        budget_entity_id: None,
        budget_limit: None,
        budget_spend: None,
        proxy_type: None,
        headers: None,
        response_headers: None,
        litellm_response_headers: None,
        traceback_lines: Vec::new(),
    }
}

pub(crate) fn error(message: &str) -> SpanError {
    SpanError {
        error_type: None,
        message: Some(message.to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    }
}
