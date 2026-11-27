#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_error::proxy_exception::{
    ExceptionCode, ModelAccessDeniedProxyException, ProxyException, ProxyExceptionInit,
};
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{CaptureMessageContent, Settings};
use betterlite_logging::otel::emitter::span_emission::MapperKind;
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::v2::RequestTraceSource;
use betterlite_logging::otel::mappers::genai::GenAIMapper;
use betterlite_logging::otel::mappers::utils::tool_attr_budget;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::exporters::{
    ExportResult, ExporterFactories, ExporterFactory, ExporterSpec, HttpClients,
};
use betterlite_logging::otel::plumbing::providers::ProviderState;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use chrono::FixedOffset;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Status, Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracer, SdkTracerProvider, SimpleSpanProcessor, SpanData,
};

use super::{ServerSpanLogger, close_error_span, exception_facts};

const CALLBACK: &str = "b10156";

struct Rig {
    exporter: InMemorySpanExporter,
    tracer: SdkTracer,
    provider: SdkTracerProvider,
}

impl Rig {
    fn new() -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
            .build();
        let tracer = provider.tracer("litellm");
        Self {
            exporter,
            tracer,
            provider,
        }
    }

    fn span(&self) -> LiveSpan {
        LiveSpan::from(self.tracer.start("dangling-server-span"))
    }

    fn finished(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }

    fn v2_logger(&self) -> ServerSpanLogger {
        let parts = construction_parts(&self.provider, &self.exporter);
        let callback = V2Callback::new(parts, &V2Registry::default()).expect("the v2 callback");
        ServerSpanLogger::V2(callback)
    }
}

fn legacy_logger() -> ServerSpanLogger {
    ServerSpanLogger::Legacy(Box::new(error_config()))
}

fn error_config() -> ErrorInformationConfig {
    ErrorInformationConfig {
        normalizer: Normalizer::new().expect("normalizer"),
        redactor: None,
        log_client_error_tracebacks: false,
    }
}

fn proxy_exception() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "test-failure".to_owned(),
        error_type: "ProxyException".to_owned(),
        param: None,
        code: ExceptionCode::Status(503),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn construction_parts(
    provider: &SdkTracerProvider,
    exporter: &InMemorySpanExporter,
) -> CallbackParts {
    let config = settings();
    let factories = memory_factories(exporter);
    let tenants = tenant_cache(&config, &factories);
    CallbackParts {
        callback_name: Some(CALLBACK.to_owned()),
        config,
        raw_config: otel_settings::Settings::default(),
        tenants,
        trace: Arc::new(FixedTrace::default()),
        providers: provider_sources(provider),
        build: ProviderBuild {
            factories,
            scope_source: Arc::new(FixedScope),
            clients: clients(),
        },
        metric: MetricSettings {
            callback_settings: Arc::new(FilterSettings),
            utc_offset: FixedOffset::east_opt(0).expect("utc"),
        },
        emitter: emitter_parts(),
        error_info: error_config(),
    }
}

fn tenant_cache(config: &Settings, factories: &Arc<ExporterFactories>) -> TenantTracerCache {
    TenantTracerCache::new(
        config.clone(),
        Some(CALLBACK.to_owned()),
        "litellm",
        TenantSources {
            factories: Arc::clone(factories),
            scope_source: Arc::new(FixedScope),
            allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        },
    )
}

fn provider_sources(provider: &SdkTracerProvider) -> ProviderSources {
    ProviderSources {
        tracer_provider: Some(provider.clone()),
        meter_provider: None,
        logger_provider: None,
        meter_state: ProviderState::Unset,
        logger_state: ProviderState::Unset,
        metrics: None,
        events: None,
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

fn settings() -> Settings {
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
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn memory_factories(exporter: &InMemorySpanExporter) -> Arc<ExporterFactories> {
    let sink = exporter.clone();
    let factory: ExporterFactory =
        Arc::new(move |_spec: &ExporterSpec| -> ExportResult<SpanSink> {
            Ok(SpanSink::InMemory(sink.clone()))
        });
    Arc::new(ExporterFactories::new(
        clients(),
        BTreeMap::from([("b10156_memory".to_owned(), factory)]),
    ))
}

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

#[derive(Debug)]
struct NoClient;

#[async_trait::async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the request error span tests export in memory"))
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

fn attribute<'span>(span: &'span SpanData, key: &str) -> Option<&'span Value> {
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| &pair.value)
}

fn text_attribute(span: &SpanData, key: &str) -> String {
    let value = attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing"));
    let Value::String(text) = value else {
        panic!("attribute {key} is not text: {value:?}");
    };
    text.as_str().to_owned()
}

fn i64_attribute(span: &SpanData, key: &str) -> i64 {
    let value = attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing"));
    let Value::I64(number) = value else {
        panic!("attribute {key} is not an integer: {value:?}");
    };
    *number
}

fn has_error_attribute(span: &SpanData) -> bool {
    span.attributes
        .iter()
        .any(|pair| pair.key.as_str().starts_with("error."))
}

#[test]
fn legacy_error_status_ends_the_span_as_an_error() {
    let rig = Rig::new();
    let mut slot = Some(rig.span());
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 503, Some(&error));

    assert!(slot.is_none(), "the legacy close clears the slot");
    let spans = rig.finished();
    assert_eq!(spans.len(), 1, "the legacy close ends exactly one span");
    let data = spans.first().expect("the close ends the span");
    assert_eq!(i64_attribute(data, "http.response.status_code"), 503);
    assert_eq!(text_attribute(data, "error.type"), "ProxyException");
    assert_eq!(text_attribute(data, "error.message"), "test-failure");
    assert_eq!(data.status, Status::error(""));
}

#[test]
fn legacy_success_status_ends_the_span_ok_without_error_attributes() {
    let rig = Rig::new();
    let mut slot = Some(rig.span());
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 200, Some(&error));

    assert!(slot.is_none(), "the legacy close clears the slot");
    let spans = rig.finished();
    let data = spans.first().expect("the close ends the span");
    assert_eq!(i64_attribute(data, "http.response.status_code"), 200);
    assert_eq!(data.status, Status::Ok);
    assert!(
        !has_error_attribute(data),
        "a success answer carries no error attributes"
    );
}

#[test]
fn legacy_four_hundred_ends_the_span_as_an_error() {
    let rig = Rig::new();
    let mut slot = Some(rig.span());
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 400, Some(&error));

    let spans = rig.finished();
    let data = spans.first().expect("the close ends the span");
    assert_eq!(data.status, Status::error(""));
    assert_eq!(i64_attribute(data, "http.response.status_code"), 400);
    assert_eq!(text_attribute(data, "error.type"), "ProxyException");
}

#[test]
fn legacy_three_hundred_ninety_nine_ends_the_span_ok() {
    let rig = Rig::new();
    let mut slot = Some(rig.span());
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 399, Some(&error));

    let spans = rig.finished();
    let data = spans.first().expect("the close ends the span");
    assert_eq!(data.status, Status::Ok);
    assert_eq!(i64_attribute(data, "http.response.status_code"), 399);
    assert!(
        !has_error_attribute(data),
        "below 400 only the status code attribute is stamped"
    );
}

#[test]
fn legacy_missing_error_ends_the_span_as_an_error_without_error_attributes() {
    let rig = Rig::new();
    let mut slot = Some(rig.span());
    let logger = legacy_logger();

    close_error_span(&mut slot, Some(&logger), false, 503, None);

    assert!(slot.is_none(), "the legacy close clears the slot");
    let spans = rig.finished();
    let data = spans.first().expect("the close ends the span");
    assert_eq!(data.status, Status::error(""));
    assert!(
        !has_error_attribute(data),
        "a missing exception stamps no error attributes"
    );
}

#[test]
fn v2_error_status_stamps_the_error_and_leaves_the_span_open() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let logger = rig.v2_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), true, 500, Some(&error));

    assert!(
        rig.finished().is_empty(),
        "the instrumentor still owns and ends the span"
    );
    assert_eq!(
        slot.as_ref().expect("the slot keeps the span").context,
        live.context
    );
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert_eq!(text_attribute(data, "error.type"), "ProxyException");
    assert!(
        attribute(data, "http.response.status_code").is_none(),
        "the instrumentor writes the status code, not this close"
    );
}

#[test]
fn v2_four_hundred_stamps_the_error_and_keeps_the_span_open() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let logger = rig.v2_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), true, 400, Some(&error));

    assert!(
        rig.finished().is_empty(),
        "the instrumentor still owns and ends the span"
    );
    assert!(slot.is_some(), "a v2 close keeps the span in the slot");
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert_eq!(
        text_attribute(data, "error.type"),
        "ProxyException",
        "400 is the first status that stamps the error"
    );
}

#[test]
fn v2_success_status_writes_nothing_and_keeps_the_span() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let logger = rig.v2_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), true, 302, Some(&error));

    assert!(
        rig.finished().is_empty(),
        "the instrumentor still owns and ends the span"
    );
    assert!(slot.is_some(), "a v2 close keeps the span in the slot");
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert!(
        data.attributes.is_empty(),
        "a redirect answer stamps nothing"
    );
    assert_eq!(data.status, Status::Unset);
}

#[test]
fn v2_legacy_logger_stamps_the_error_and_keeps_the_span_open() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), true, 500, Some(&error));

    assert!(
        rig.finished().is_empty(),
        "the instrumentor still owns and ends the span"
    );
    assert!(slot.is_some(), "a v2 close keeps the span in the slot");
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert_eq!(text_attribute(data, "error.type"), "ProxyException");
    assert_eq!(
        data.status,
        Status::Unset,
        "the legacy class does not set the status under v2"
    );
}

#[test]
fn skip_empty_slot_stays_empty() {
    let rig = Rig::new();
    let mut slot = None;
    let logger = legacy_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 503, Some(&error));

    assert!(slot.is_none(), "an empty slot stays empty");
    assert!(rig.finished().is_empty(), "an empty slot ends no span");
}

#[test]
fn skip_missing_logger_keeps_the_span_open_in_the_slot() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, None, false, 503, Some(&error));

    assert!(rig.finished().is_empty(), "no logger means no close");
    assert_eq!(
        slot.as_ref().expect("the slot keeps the span").context,
        live.context
    );
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert!(data.attributes.is_empty(), "no logger stamps nothing");
    assert_eq!(data.status, Status::Unset);
}

#[test]
fn skip_v2_logger_with_v2_off_clears_the_slot_and_leaves_the_span_open() {
    let rig = Rig::new();
    let live = rig.span();
    let mut slot = Some(live.clone());
    let logger = rig.v2_logger();
    let error = exception_facts("ProxyException", &proxy_exception());

    close_error_span(&mut slot, Some(&logger), false, 503, Some(&error));

    assert!(slot.is_none(), "the finally clause clears the slot");
    assert!(
        rig.finished().is_empty(),
        "the v2 logger's span is left open"
    );
    live.lock().end();
    let spans = rig.finished();
    let data = spans.first().expect("the test ends the span");
    assert!(
        data.attributes.is_empty(),
        "the failing attribute write stamps nothing"
    );
    assert_eq!(data.status, Status::Unset);
}

#[test]
fn facts_keep_the_class_message_code_and_headers() {
    let error = ProxyException::new(ProxyExceptionInit {
        message: "test-failure".to_owned(),
        error_type: "ProxyException".to_owned(),
        param: Some("model".to_owned()),
        code: ExceptionCode::Status(503),
        headers: BTreeMap::from([
            ("x-request-id".to_owned(), "abc".to_owned()),
            ("x-b".to_owned(), "2".to_owned()),
        ]),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });

    let facts = exception_facts("ProxyException", &error);

    assert_eq!(facts.class_name, "ProxyException");
    assert_eq!(facts.message.as_deref(), Some("test-failure"));
    assert_eq!(facts.code.as_deref(), Some("503"));
    let headers = facts.headers.as_ref().expect("the facts carry the headers");
    assert_eq!(
        headers.keys().cloned().collect::<Vec<String>>(),
        vec!["x-b".to_owned(), "x-request-id".to_owned()],
        "the headers keep the ProxyException's key order"
    );
    assert_eq!(headers.get("x-request-id").map(String::as_str), Some("abc"));
    assert!(facts.class.is_none());
    assert!(facts.status_code.is_none());
    assert!(facts.response_status_code.is_none());
    assert!(facts.llm_provider.is_none());
    assert!(facts.proxy_type.is_none());
    assert!(facts.traceback_lines.is_empty());
}

#[test]
fn facts_keep_a_none_code_and_the_access_denied_class() {
    let denied = ModelAccessDeniedProxyException::new(
        "test-failure".to_owned(),
        "the key may not call this model".to_owned(),
        "ProxyException".to_owned(),
        None,
        ExceptionCode::Absent,
    );

    let facts = exception_facts(
        "ModelAccessDeniedProxyException",
        denied.as_proxy_exception(),
    );

    assert_eq!(facts.class_name, "ModelAccessDeniedProxyException");
    assert_eq!(facts.code.as_deref(), Some("None"));
    assert_eq!(facts.message.as_deref(), Some("test-failure"));
}
