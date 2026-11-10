#![cfg(test)]

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use opentelemetry::trace::{
    Link, Span as _, SpanContext, SpanId, SpanKind, Status, TraceContextExt as _, TraceFlags,
    TraceId, TraceState, Tracer as _, TracerProvider as _,
};
use opentelemetry::{Context, Value};
use opentelemetry_sdk::logs::InMemoryLogExporter;
use opentelemetry_sdk::trace::SpanData as ExportedSpan;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SdkTracerProvider, SpanLimits};

use super::DedupCache;
use crate::otel::emitter::span_emission::{
    DEFAULT_SPAN_ATTRIBUTE_LIMIT, EmitOptions, MapperKind, SpanEmitter, StartOptions,
};
use crate::otel::emitter::span_kind::span_kind;
use crate::otel::mappers::base::{AttributeMap, SpanData};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::legacy::LegacyMapper;
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
    mappers: Vec<MapperKind>,
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

fn emitter_with_event_recorder() -> (
    InMemoryLogExporter,
    InMemorySpanExporter,
    SpanEmitter<SdkTracer>,
) {
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
    (log_exporter, exporter, emitter)
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

fn fixed(entries: &[(&str, &str)]) -> AttributeMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::from((*value).to_owned())))
        .collect()
}

fn indexed(count: usize) -> AttributeMap {
    let mut attributes = AttributeMap::new();
    for index in 0..count {
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

fn rate_limit_error(code: Option<&str>) -> SpanError {
    SpanError {
        error_type: Some("RateLimitError".to_owned()),
        message: Some("slow down".to_owned()),
        code: code.map(str::to_owned),
        stack_trace: None,
        llm_provider: None,
    }
}

fn bare_emitter(limit: Option<usize>) -> SpanEmitter<SdkTracer> {
    let (_exporter, provider) = exporter_and_provider();
    emitter(provider.tracer("span-emission"), Vec::new(), limit)
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
    let finished = exported(&exporter);
    assert_eq!(finished.len(), 3);
    let names: Vec<&str> = finished.iter().map(|span| span.name.as_ref()).collect();
    assert!(names.contains(&"chat gpt-4o"));
    assert!(names.contains(&"tools/call get-weather"));
    assert!(names.contains(&"redis get"));
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

fn service_attributes_through(mappers: Vec<MapperKind>) -> BTreeMap<String, Value> {
    let (exporter, provider) = exporter_and_provider();
    let mut service = service_data();
    service.event_metadata = BTreeMap::from([(
        "litellm.service.name".to_owned(),
        "from-legacy-metadata".to_owned(),
    )]);
    let emitter = emitter(provider.tracer("span-emission"), mappers, None);
    emitter.emit(
        SpanRole::Service,
        &SpanData::Service(&service),
        &options(None, None),
    );
    attributes(&exported(&exporter)[0])
}

#[test]
fn emit_runs_every_mapper_and_the_later_one_wins() {
    let written = service_attributes_through(vec![
        MapperKind::GenAI(GenAIMapper {
            tool_attr_budget: 8,
        }),
        MapperKind::Legacy(LegacyMapper {
            tool_attr_budget: 8,
        }),
    ]);
    assert_eq!(
        written.get("litellm.service.name"),
        Some(&Value::from("from-legacy-metadata"))
    );
    assert_eq!(
        written.get("litellm.service.call_type"),
        Some(&Value::from("get"))
    );
}

#[test]
fn emit_lets_the_genai_mapper_win_when_it_runs_last() {
    let written = service_attributes_through(vec![
        MapperKind::Legacy(LegacyMapper {
            tool_attr_budget: 8,
        }),
        MapperKind::GenAI(GenAIMapper {
            tool_attr_budget: 8,
        }),
    ]);
    assert_eq!(
        written.get("litellm.service.name"),
        Some(&Value::from("redis"))
    );
}

#[test]
fn emit_sheds_whole_message_groups_least_valuable_first() {
    let fitted = bare_emitter(Some(6)).fit_to_budget(indexed(5), None);
    assert_eq!(fitted.len(), 6);
    assert!(!fitted.contains_key("llm.input_messages.1.message.role"));
    assert!(!fitted.contains_key("llm.input_messages.2.message.content"));
    assert!(fitted.contains_key("llm.input_messages.0.message.role"));
    assert!(fitted.contains_key("llm.input_messages.3.message.content"));
    assert!(fitted.contains_key("llm.input_messages.4.message.content"));
}

#[test]
fn emit_reserves_room_for_error_attributes() {
    let error = rate_limit_error(Some("429"));
    let fitted = bare_emitter(Some(10)).fit_to_budget(indexed(5), Some(&error));
    assert_eq!(fitted.len(), 6);
    assert!(!fitted.contains_key("llm.input_messages.1.message.content"));
    assert!(!fitted.contains_key("llm.input_messages.2.message.content"));
}

#[test]
fn emit_keeps_every_attribute_without_an_attribute_limit() {
    let error = rate_limit_error(Some("429"));
    let fitted = bare_emitter(None).fit_to_budget(indexed(50), Some(&error));
    assert_eq!(fitted.len(), 100);
}

#[test]
fn emit_never_sheds_attributes_that_are_not_indexed_messages() {
    let mut mapped = fixed(&[
        ("litellm.one", "1"),
        ("litellm.two", "2"),
        ("litellm.three", "3"),
    ]);
    mapped.extend(indexed(2));
    let fitted = bare_emitter(Some(1)).fit_to_budget(mapped, None);
    assert_eq!(fitted.len(), 3);
    assert!(fitted.contains_key("litellm.three"));
    assert!(!fitted.contains_key("llm.input_messages.0.message.role"));
}

#[test]
fn an_error_mapper_attribute_is_not_reserved_twice() {
    let mut mapped = fixed(&[("error.type", "MappedError")]);
    mapped.extend(indexed(2));
    let error = rate_limit_error(Some("429"));
    let roomy = bare_emitter(Some(7)).fit_to_budget(mapped.clone(), Some(&error));
    assert_eq!(roomy.len(), 5);
    let tight = bare_emitter(Some(6)).fit_to_budget(mapped, Some(&error));
    assert_eq!(tight.len(), 3);
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
    let span = &exported(&exporter)[0];
    assert_eq!(span.name, "execute_guardrail guardrail");
    let written = attributes(span);
    assert_eq!(
        written.get("error.type"),
        Some(&Value::from("guardrail_intervened"))
    );
    assert_eq!(written.get("error.message"), Some(&Value::from("blocked")));
}

#[test]
fn an_mcp_error_never_reaches_the_event_recorder() {
    let (log_exporter, exporter, emitter) = emitter_with_event_recorder();
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
    let (log_exporter, exporter, emitter) = emitter_with_event_recorder();
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
    assert_eq!(span.end_time, end);
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
    let finished = exported(&exporter);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].name, "tools/list");
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
fn emit_parents_the_span_to_the_context_it_is_given() {
    let (exporter, provider) = exporter_and_provider();
    let tracer = provider.tracer("span-emission");
    let mut root = tracer.start("root");
    let parent = Context::new().with_remote_span_context(root.span_context().clone());
    let emitter = emitter(tracer, Vec::new(), None);
    let data = call_data("call-1");
    let options = EmitOptions {
        parent: Some(&parent),
        start_time: None,
        end_time: None,
        tracer: None,
        links: &[],
    };
    emitter.emit(SpanRole::LlmCall, &SpanData::LLMCall(&data), &options);
    root.end();
    let finished = exported(&exporter);
    let child = finished
        .iter()
        .find(|span| span.name == "chat gpt-4o")
        .expect("child");
    assert_eq!(child.parent_span_id, parent.span().span_context().span_id());
}

#[test]
fn a_per_request_tracer_override_creates_the_span() {
    let (default_exporter, default_provider) = exporter_and_provider();
    let (override_exporter, override_provider) = exporter_and_provider();
    let emitter = emitter(default_provider.tracer("default"), Vec::new(), None);
    let override_tracer = override_provider.tracer("override");
    let options = EmitOptions {
        parent: None,
        start_time: None,
        end_time: None,
        tracer: Some(&override_tracer),
        links: &[],
    };
    let data = call_data("call-1");
    emitter.emit(SpanRole::LlmCall, &SpanData::LLMCall(&data), &options);
    assert!(exported(&default_exporter).is_empty());
    assert_eq!(exported(&override_exporter).len(), 1);
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
    let (exporter, provider) = exporter_and_provider();
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(BTreeMap::from([(
        "DATABASE_URL".to_owned(),
        "postgresql://user:pass@db.example.com:5433/app".to_owned(),
    )]));
    let emitter = SpanEmitter {
        tracer: provider.tracer("span-emission"),
        environment,
        mappers: vec![MapperKind::GenAI(GenAIMapper {
            tool_attr_budget: 8,
        })],
        event_recorder: None,
        attribute_limit: None,
        emitted: DedupCache::default(),
    };
    let mut service = service_data();
    service.service_name = "postgres".to_owned();
    service.call_type = Some("postgres_query".to_owned());
    emitter.emit(
        SpanRole::Service,
        &SpanData::Service(&service),
        &options(None, None),
    );
    assert_eq!(
        attributes(&exported(&exporter)[0]).get("server.address"),
        Some(&Value::from("db.example.com"))
    );
}
