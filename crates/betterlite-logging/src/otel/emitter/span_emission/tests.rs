#![cfg(test)]

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use opentelemetry::Value;
use opentelemetry::trace::{
    Link, Span as _, SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
    TracerProvider as _,
};
use opentelemetry_sdk::logs::InMemoryLogExporter;
use opentelemetry_sdk::trace::SpanData as ExportedSpan;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SdkTracerProvider, SpanLimits};

use super::DedupCache;
use crate::otel::emitter::span_emission::{
    DEFAULT_SPAN_ATTRIBUTE_LIMIT, EmitOptions, SpanEmitter, StartOptions,
};
use crate::otel::emitter::span_kind::span_kind;
use crate::otel::mappers::base::{AttributeMap, AttributeMapper, SpanData};
use crate::otel::model::metadata::wire::decode;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::{MCPListToolsSpanData, MCPToolCallSpanData};
use crate::otel::model::payloads::service::ServiceSpanData;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::{GenAIEventRecorder, SystemClock};
use crate::otel::plumbing::providers::{build_logger_provider, get_event_logger};
use crate::otel::plumbing::sinks::LogSink;
use crate::otel::vocabulary::constants::MCPMethod;
use crate::otel::vocabulary::registry::SpanRole;

fn chat_payload(call_id: &str) -> Payload {
    let text = format!(
        r#"{{"id": "{call_id}", "model": "gpt-4o", "call_type": "completion", "custom_llm_provider": "openai"}}"#
    );
    decode::<Payload>(&text).expect("payload")
}

fn call_data(call_id: &str) -> LLMCallSpanData {
    let options = LLMCallOptions {
        capture_content: false,
        time_to_first_chunk_seconds: None,
        request_route: None,
        trace: TraceControls::default(),
        session_id: None,
    };
    LLMCallSpanData::from_standard_logging_payload(&chat_payload(call_id), options).expect("span")
}

fn mcp_data(call_id: &str) -> MCPToolCallSpanData {
    let text = format!(
        r#"{{"id": "{call_id}", "model": "gpt-4o", "call_type": "call_mcp_tool", "metadata": {{"mcp_tool_call_metadata": {{"name": "get-weather"}}}}}}"#
    );
    let payload = decode::<Payload>(&text).expect("payload");
    MCPToolCallSpanData::from_standard_logging_payload(&payload, false).expect("mcp data")
}

fn service_data() -> ServiceSpanData {
    ServiceSpanData {
        service_name: "redis".to_owned(),
        call_type: Some("get".to_owned()),
        caller: None,
        error: None,
        event_metadata: BTreeMap::new(),
    }
}

fn exporter_and_provider() -> (InMemorySpanExporter, SdkTracerProvider) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (exporter, provider)
}

fn emitter(
    tracer: SdkTracer,
    mappers: Vec<Box<dyn AttributeMapper + Send + Sync>>,
    limit: Option<usize>,
) -> SpanEmitter<SdkTracer> {
    SpanEmitter {
        tracer,
        environment: Arc::new(BTreeMap::<String, String>::new()),
        mappers,
        event_recorder: None,
        attribute_limit: limit,
        emitted: DedupCache::default(),
    }
}

fn options<'source>(
    start: Option<SystemTime>,
    end: Option<SystemTime>,
) -> EmitOptions<'source, SdkTracer> {
    EmitOptions {
        parent: None,
        start_time: start,
        end_time: end,
        tracer: None,
        links: &[],
    }
}

fn exported(exporter: &InMemorySpanExporter) -> Vec<ExportedSpan> {
    exporter.get_finished_spans().expect("spans")
}

fn attributes(span: &ExportedSpan) -> BTreeMap<String, Value> {
    span.attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

struct FixedAttributes(Vec<(&'static str, &'static str)>);

impl AttributeMapper for FixedAttributes {
    fn map(&self, _data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        self.0
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Value::from(*value)))
            .collect()
    }
}

struct IndexedMessages(usize);

impl AttributeMapper for IndexedMessages {
    fn map(&self, _data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        let mut attributes = AttributeMap::new();
        for index in 0..self.0 {
            attributes.insert(
                format!("llm.input_messages.{index}.message.role"),
                Value::from("user"),
            );
            attributes.insert(
                format!("llm.input_messages.{index}.message.content"),
                Value::from("hi"),
            );
        }
        attributes
    }
}

#[test]
fn emit_dedups_the_second_span_for_the_same_call_and_role() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let data = call_data("call-1");
    let span_data = SpanData::LLMCall(&data);
    assert!(
        emitter
            .emit(SpanRole::LlmCall, &span_data, &options(None, None))
            .is_some()
    );
    assert!(
        emitter
            .emit(SpanRole::LlmCall, &span_data, &options(None, None))
            .is_none()
    );
    assert_eq!(exported(&exporter).len(), 1);
}

#[test]
fn one_call_id_does_not_dedup_across_roles() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let call = call_data("call-1");
    let mcp = mcp_data("call-1");
    let service = service_data();
    assert!(
        emitter
            .emit(
                SpanRole::LlmCall,
                &SpanData::LLMCall(&call),
                &options(None, None)
            )
            .is_some()
    );
    assert!(
        emitter
            .emit(
                SpanRole::McpToolCall,
                &SpanData::MCPToolCall(&mcp),
                &options(None, None)
            )
            .is_some()
    );
    assert!(
        emitter
            .emit(
                SpanRole::Service,
                &SpanData::Service(&service),
                &options(None, None)
            )
            .is_some()
    );
    assert_eq!(exported(&exporter).len(), 3);
}

#[test]
fn a_call_without_an_id_is_never_deduped() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let service = service_data();
    assert!(
        emitter
            .emit(
                SpanRole::Service,
                &SpanData::Service(&service),
                &options(None, None)
            )
            .is_some()
    );
    assert!(
        emitter
            .emit(
                SpanRole::Service,
                &SpanData::Service(&service),
                &options(None, None)
            )
            .is_some()
    );
    assert_eq!(exported(&exporter).len(), 2);
}

#[test]
fn an_empty_call_id_is_not_a_dedup_key() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut data = call_data("call-1");
    data.identity.call_id = Some(String::new());
    let span_data = SpanData::LLMCall(&data);
    assert!(
        emitter
            .emit(SpanRole::LlmCall, &span_data, &options(None, None))
            .is_some()
    );
    assert!(
        emitter
            .emit(SpanRole::LlmCall, &span_data, &options(None, None))
            .is_some()
    );
    assert_eq!(exported(&exporter).len(), 2);
}

#[test]
fn mark_emitted_registers_a_span_emitted_outside_the_engine() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    assert!(!emitter.mark_emitted(Some("call-1"), SpanRole::LlmCall));
    let data = call_data("call-1");
    assert!(
        emitter
            .emit(
                SpanRole::LlmCall,
                &SpanData::LLMCall(&data),
                &options(None, None)
            )
            .is_none()
    );
    assert!(exported(&exporter).is_empty());
}

#[test]
fn the_dedup_cache_evicts_the_least_recently_used_key() {
    let cache = DedupCache::default();
    let id_count = super::DEDUP_CACHE_MAX;
    (0..id_count).for_each(|index| {
        assert!(!cache.mark(&format!("call-{index}"), SpanRole::LlmCall));
    });
    assert!(cache.mark("call-0", SpanRole::LlmCall));
    assert!(!cache.mark("call-extra", SpanRole::LlmCall));
    assert!(!cache.mark("call-1", SpanRole::LlmCall));
    assert!(cache.mark("call-0", SpanRole::LlmCall));
    assert!(cache.mark("call-extra", SpanRole::LlmCall));
}

#[test]
fn emit_runs_every_mapper_and_the_last_writer_wins() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![
        Box::new(FixedAttributes(vec![
            ("litellm.shared", "first"),
            ("litellm.one", "1"),
        ])),
        Box::new(FixedAttributes(vec![
            ("litellm.shared", "second"),
            ("litellm.two", "2"),
        ])),
    ];
    let emitter = emitter(provider.tracer("span-emission"), mappers, None);
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(written.get("litellm.one"), Some(&Value::from("1")));
    assert_eq!(written.get("litellm.two"), Some(&Value::from("2")));
    assert_eq!(written.get("litellm.shared"), Some(&Value::from("second")));
}

#[test]
fn emit_sheds_whole_message_groups_least_valuable_first() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![Box::new(IndexedMessages(5))];
    let emitter = emitter(provider.tracer("span-emission"), mappers, Some(6));
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(written.len(), 6);
    assert!(!written.contains_key("llm.input_messages.1.message.role"));
    assert!(!written.contains_key("llm.input_messages.2.message.content"));
    assert!(written.contains_key("llm.input_messages.0.message.role"));
    assert!(written.contains_key("llm.input_messages.3.message.content"));
    assert!(written.contains_key("llm.input_messages.4.message.content"));
}

#[test]
fn emit_reserves_room_for_error_attributes() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![Box::new(IndexedMessages(5))];
    let emitter = emitter(provider.tracer("span-emission"), mappers, Some(10));
    let mut data = call_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: Some("429".to_owned()),
        stack_trace: None,
        llm_provider: None,
    });
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(written.len(), 9);
    assert!(!written.contains_key("llm.input_messages.1.message.content"));
    assert!(!written.contains_key("llm.input_messages.2.message.content"));
    assert_eq!(
        written.get("error.type"),
        Some(&Value::from("RateLimitError"))
    );
    assert_eq!(
        written.get("error.message"),
        Some(&Value::from("slow down"))
    );
}

#[test]
fn emit_keeps_every_attribute_without_an_attribute_limit() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![Box::new(IndexedMessages(50))];
    let emitter = emitter(provider.tracer("span-emission"), mappers, None);
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    assert_eq!(attributes(&exported(&exporter)[0]).len(), 100);
}

#[test]
fn emit_never_sheds_attributes_that_are_not_indexed_messages() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![
        Box::new(FixedAttributes(vec![
            ("litellm.one", "1"),
            ("litellm.two", "2"),
            ("litellm.three", "3"),
        ])),
        Box::new(IndexedMessages(2)),
    ];
    let emitter = emitter(provider.tracer("span-emission"), mappers, Some(1));
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(written.len(), 3);
    assert!(written.contains_key("litellm.three"));
    assert!(!written.contains_key("llm.input_messages.0.message.role"));
}

#[test]
fn an_error_mapper_attribute_is_not_reserved_twice() {
    let (exporter, provider) = exporter_and_provider();
    let mappers: Vec<Box<dyn AttributeMapper + Send + Sync>> = vec![Box::new(FixedAttributes(
        vec![("error.type", "MappedError")],
    ))];
    let emitter = emitter(provider.tracer("span-emission"), mappers, Some(2));
    let mut data = call_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    });
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(
        written.get("error.type"),
        Some(&Value::from("RateLimitError"))
    );
    assert_eq!(
        written.get("error.message"),
        Some(&Value::from("slow down"))
    );
}

#[test]
fn finish_span_stamps_the_error_attributes_and_status() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut data = call_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: Some("429".to_owned()),
        stack_trace: Some("frame one".to_owned()),
        llm_provider: Some("openai".to_owned()),
    });
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let span = &exported(&exporter)[0];
    let written = attributes(span);
    assert_eq!(
        written.get("error.type"),
        Some(&Value::from("RateLimitError"))
    );
    assert_eq!(
        written.get("litellm.provider.error.code"),
        Some(&Value::from("429"))
    );
    assert_eq!(
        written.get("litellm.provider.error.stack_trace"),
        Some(&Value::from("frame one"))
    );
    assert_eq!(
        written.get("litellm.provider.error.llm_provider"),
        Some(&Value::from("openai"))
    );
    assert_eq!(span.status, Status::error("slow down"));
}

#[test]
fn finish_span_records_the_exception_event() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut data = call_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    });
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let span = &exported(&exporter)[0];
    assert_eq!(span.events.events.len(), 1);
    let event = &span.events.events[0];
    assert_eq!(event.name, "exception");
    let event_attributes: BTreeMap<String, Value> = event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    assert_eq!(
        event_attributes.get("exception.type"),
        Some(&Value::from("RateLimitError"))
    );
    assert_eq!(
        event_attributes.get("exception.message"),
        Some(&Value::from("slow down"))
    );
}

#[test]
fn a_successful_span_keeps_its_status_unset() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let span = &exported(&exporter)[0];
    assert_eq!(span.status, Status::Unset);
    assert!(span.events.events.is_empty());
}

#[test]
fn an_error_without_a_type_or_message_stamps_nothing() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut data = call_data("call-1");
    data.error = Some(SpanError::default());
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    let span = &exported(&exporter)[0];
    assert_eq!(span.status, Status::Unset);
    assert!(span.events.events.is_empty());
    assert!(!attributes(span).contains_key("error.type"));
}

#[test]
fn a_guardrail_error_reaches_the_span() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut entry: BTreeMap<String, OtelModelValue> = BTreeMap::new();
    entry.insert(
        "guardrail_status".to_owned(),
        OtelModelValue::String("guardrail_intervened".to_owned()),
    );
    entry.insert(
        "guardrail_action".to_owned(),
        OtelModelValue::String("blocked".to_owned()),
    );
    let data = GuardrailSpanData::from_logging_entry(&entry);
    emitter.emit(
        SpanRole::Guardrail,
        &SpanData::Guardrail(&data),
        &options(None, None),
    );
    let written = attributes(&exported(&exporter)[0]);
    assert_eq!(
        written.get("error.type"),
        Some(&Value::from("guardrail_intervened"))
    );
    assert_eq!(written.get("error.message"), Some(&Value::from("blocked")));
}

#[test]
fn an_mcp_error_never_reaches_the_event_recorder() {
    let log_exporter = InMemoryLogExporter::default();
    let logger_provider = build_logger_provider(
        &OtelConfig::default(),
        LogSink::InMemory(log_exporter.clone()),
    );
    let recorder = GenAIEventRecorder {
        event_logger: get_event_logger(&logger_provider, "litellm"),
        clock: Box::new(SystemClock),
    };
    let (exporter, provider) = exporter_and_provider();
    let emitter = SpanEmitter {
        tracer: provider.tracer("span-emission"),
        environment: Arc::new(BTreeMap::<String, String>::new()),
        mappers: Vec::new(),
        event_recorder: Some(Arc::new(recorder)),
        attribute_limit: None,
        emitted: DedupCache::default(),
    };
    let mut data = mcp_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("MCPError".to_owned()),
        message: None,
        code: None,
        stack_trace: None,
        llm_provider: None,
    });
    emitter.emit(
        SpanRole::McpToolCall,
        &SpanData::MCPToolCall(&data),
        &options(None, None),
    );
    assert_eq!(exported(&exporter).len(), 1);
    assert!(log_exporter.get_emitted_logs().expect("logs").is_empty());
}

#[test]
fn an_llm_call_error_reaches_the_event_recorder() {
    let log_exporter = InMemoryLogExporter::default();
    let logger_provider = build_logger_provider(
        &OtelConfig::default(),
        LogSink::InMemory(log_exporter.clone()),
    );
    let recorder = GenAIEventRecorder {
        event_logger: get_event_logger(&logger_provider, "litellm"),
        clock: Box::new(SystemClock),
    };
    let (exporter, provider) = exporter_and_provider();
    let emitter = SpanEmitter {
        tracer: provider.tracer("span-emission"),
        environment: Arc::new(BTreeMap::<String, String>::new()),
        mappers: Vec::new(),
        event_recorder: Some(Arc::new(recorder)),
        attribute_limit: None,
        emitted: DedupCache::default(),
    };
    let mut data = call_data("call-1");
    data.error = Some(SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    });
    let end = UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("time");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, Some(end)),
    );
    let span = &exported(&exporter)[0];
    let logs = log_exporter.get_emitted_logs().expect("logs");
    let record = &logs[0].record;
    let context = record.trace_context().expect("trace context");
    assert_eq!(context.trace_id, span.span_context.trace_id());
    assert_eq!(record.timestamp(), Some(end));
}

#[test]
fn finish_span_rebuilds_the_name_from_the_payload() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let mut data = call_data("call-1");
    let start = StartOptions {
        parent: None,
        start_time: None,
        tracer: None,
        links: &[],
    };
    let mut span = emitter.start_span(SpanRole::LlmCall, "provisional", &start);
    data.request_model = "gpt-4o-mini".to_owned();
    emitter.finish_span(
        SpanRole::LlmCall,
        &mut span,
        &SpanData::LLMCall(&data),
        None,
    );
    assert_eq!(exported(&exporter)[0].name, "chat gpt-4o-mini");
}

#[test]
fn the_default_attribute_limit_matches_the_sdk() {
    let limits = SpanLimits::default();
    assert_eq!(
        usize::try_from(limits.max_attributes_per_span).expect("limit"),
        DEFAULT_SPAN_ATTRIBUTE_LIMIT
    );
}

fn assert_send_sync<T>(_marker: PhantomData<T>)
where
    T: Send + Sync,
{
}

#[test]
fn the_emitter_is_shareable_across_request_tasks() {
    assert_send_sync(PhantomData::<SpanEmitter<SdkTracer>>);
    assert_send_sync(PhantomData::<DedupCache>);
}

#[test]
fn an_mcp_list_tools_span_dedups_on_its_call_id() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let data = MCPListToolsSpanData {
        method: MCPMethod::ToolsList,
        session_id: None,
        error: None,
        identity: call_data("call-2").identity,
    };
    assert!(
        emitter
            .emit(
                SpanRole::McpListTools,
                &SpanData::MCPListTools(&data),
                &options(None, None)
            )
            .is_some()
    );
    assert!(
        emitter
            .emit(
                SpanRole::McpListTools,
                &SpanData::MCPListTools(&data),
                &options(None, None)
            )
            .is_none()
    );
    assert_eq!(exported(&exporter).len(), 1);
}

#[test]
fn the_span_kind_follows_the_role_registry() {
    assert_eq!(span_kind(SpanRole::LlmCall), SpanKind::Client);
    assert_eq!(span_kind(SpanRole::Service), SpanKind::Internal);
    assert_eq!(span_kind(SpanRole::ProxyRequest), SpanKind::Server);
}

#[test]
fn start_span_applies_the_kind_and_the_start_time() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let start_time = UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("time");
    let start = StartOptions {
        parent: None,
        start_time: Some(start_time),
        tracer: None,
        links: &[],
    };
    let mut span = emitter.start_span(SpanRole::Service, "redis get", &start);
    span.end();
    let exported_span = &exported(&exporter)[0];
    assert_eq!(exported_span.span_kind, SpanKind::Internal);
    assert_eq!(exported_span.start_time, start_time);
    assert_eq!(exported_span.name, "redis get");
}

#[test]
fn emit_attaches_the_links_it_is_given() {
    let (exporter, provider) = exporter_and_provider();
    let emitter = emitter(provider.tracer("span-emission"), Vec::new(), None);
    let linked = SpanContext::new(
        TraceId::from(7_u128),
        SpanId::from(9_u64),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let links = [Link::with_context(linked)];
    let data = call_data("call-1");
    let options = EmitOptions {
        parent: None,
        start_time: None,
        end_time: None,
        tracer: None,
        links: &links,
    };
    emitter.emit(SpanRole::LlmCall, &SpanData::LLMCall(&data), &options);
    let span = &exported(&exporter)[0];
    assert_eq!(span.links.links.len(), 1);
    assert_eq!(
        span.links.links[0].span_context.trace_id(),
        TraceId::from(7_u128)
    );
}

#[test]
fn an_invalid_link_is_dropped() {
    use opentelemetry::trace::Tracer as _;
    let (exporter, provider) = exporter_and_provider();
    let tracer = provider.tracer("span-emission");
    let invalid = Link::with_context(SpanContext::NONE);
    let mut span = tracer
        .span_builder("linked")
        .with_links(vec![invalid])
        .start(&tracer);
    span.end();
    assert!(exported(&exporter)[0].links.links.is_empty());
}

#[test]
fn the_emitter_environment_reaches_the_mappers() {
    struct EnvironmentAttribute;
    impl AttributeMapper for EnvironmentAttribute {
        fn map(&self, _data: &SpanData<'_>, environment: &dyn EnvironmentLookup) -> AttributeMap {
            let mut attributes = AttributeMap::new();
            if let Some(value) = environment.lookup("LITELLM_TEST_VALUE") {
                attributes.insert("litellm.test".to_owned(), Value::from(value));
            }
            attributes
        }
    }
    let (exporter, provider) = exporter_and_provider();
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(BTreeMap::from([(
        "LITELLM_TEST_VALUE".to_owned(),
        "from-environment".to_owned(),
    )]));
    let emitter = SpanEmitter {
        tracer: provider.tracer("span-emission"),
        environment,
        mappers: vec![Box::new(EnvironmentAttribute)],
        event_recorder: None,
        attribute_limit: None,
        emitted: DedupCache::default(),
    };
    let data = call_data("call-1");
    emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &options(None, None),
    );
    assert_eq!(
        attributes(&exported(&exporter)[0]).get("litellm.test"),
        Some(&Value::from("from-environment"))
    );
}
