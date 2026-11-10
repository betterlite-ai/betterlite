use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use indexmap::IndexMap;
use opentelemetry::trace::{Link, Span as _, SpanBuilder, Tracer};
use opentelemetry::{Context, KeyValue};

use crate::otel::emitter::error_stamping::{ErrorStamp, error_attributes, stamp_error};
use crate::otel::emitter::message_fitting::fit_indexed_messages;
use crate::otel::emitter::span_kind::span_kind;
use crate::otel::mappers::base::{AttributeMap, AttributeMapper, SpanData};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::legacy::LegacyMapper;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::events::{GenAIEventRecorder, OperationException};
use crate::otel::vocabulary::registry::SpanRole;
use crate::otel::vocabulary::span_names::NamedSpan;

pub const DEDUP_CACHE_MAX: usize = 10_000;
// the OpenTelemetry SDK's default attribute count limit, which the Rust SDK never reports per span
pub const DEFAULT_SPAN_ATTRIBUTE_LIMIT: usize = 128;

pub struct StartOptions<'source, T> {
    pub parent: Option<&'source Context>,
    pub start_time: Option<SystemTime>,
    pub tracer: Option<&'source T>,
    pub links: &'source [Link],
}

pub struct EmitOptions<'source, T> {
    pub parent: Option<&'source Context>,
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
    pub tracer: Option<&'source T>,
    pub links: &'source [Link],
}

// the emitted (call_id, role) markers of one emitter, oldest evicted past the cap
#[derive(Debug, Default)]
pub struct DedupCache {
    seen: Mutex<IndexMap<(String, String), ()>>,
}

impl DedupCache {
    #[must_use]
    pub fn mark(&self, call_id: &str, role: SpanRole) -> bool {
        let marker = (call_id.to_owned(), role.to_string());
        let mut seen = match self.seen.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if seen.shift_remove(&marker).is_some() {
            seen.insert(marker, ());
            return true;
        }
        seen.insert(marker, ());
        if seen.len() > DEDUP_CACHE_MAX {
            seen.shift_remove_index(0);
        }
        false
    }
}

// the attribute mappers the chain can hold; a new mapper adds a variant and the match below stops compiling
pub enum MapperKind {
    GenAI(GenAIMapper),
    Legacy(LegacyMapper),
}

impl AttributeMapper for MapperKind {
    fn map(&self, data: &SpanData<'_>, environment: &dyn EnvironmentLookup) -> AttributeMap {
        match self {
            Self::GenAI(mapper) => mapper.map(data, environment),
            Self::Legacy(mapper) => mapper.map(data, environment),
        }
    }
}

// litellm.integrations.otel.emitter.SpanEmitter; the mapper chain and event recorder are injected
pub struct SpanEmitter<T> {
    pub tracer: T,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub mappers: Vec<MapperKind>,
    pub event_recorder: Option<Arc<GenAIEventRecorder>>,
    pub attribute_limit: Option<usize>,
    pub emitted: DedupCache,
}

impl<T> SpanEmitter<T>
where
    T: Tracer,
{
    pub fn start_span(&self, role: SpanRole, name: &str, options: &StartOptions<'_, T>) -> T::Span {
        let tracer = options.tracer.unwrap_or(&self.tracer);
        let mut builder = SpanBuilder::from_name(name.to_owned()).with_kind(span_kind(role));
        if let Some(start_time) = options.start_time {
            builder = builder.with_start_time(start_time);
        }
        if !options.links.is_empty() {
            builder = builder.with_links(options.links.to_vec());
        }
        let parent = options.parent.cloned().unwrap_or_else(Context::current);
        tracer.build_with_context(builder, &parent)
    }

    #[must_use]
    pub fn mark_emitted(&self, dedup_key: Option<&str>, role: SpanRole) -> bool {
        let Some(call_id) = dedup_key.filter(|key| !key.is_empty()) else {
            return false;
        };
        self.emitted.mark(call_id, role)
    }

    pub fn emit(
        &self,
        role: SpanRole,
        data: &SpanData<'_>,
        options: &EmitOptions<'_, T>,
    ) -> Option<T::Span> {
        if self.mark_emitted(span_facts(data).call_id, role) {
            return None;
        }
        let start = StartOptions {
            parent: options.parent,
            start_time: options.start_time,
            tracer: options.tracer,
            links: options.links,
        };
        let mut span = self.start_span(role, &span_name(data), &start);
        self.finish_span(role, &mut span, data, options.end_time);
        Some(span)
    }

    pub fn finish_span(
        &self,
        role: SpanRole,
        span: &mut T::Span,
        data: &SpanData<'_>,
        end_time: Option<SystemTime>,
    ) {
        span.update_name(span_name(data));
        let error = span_facts(data).error;
        for (key, value) in self.fit_to_budget(self.mapped_attributes(data), error) {
            span.set_attribute(KeyValue::new(key, value));
        }
        if let Some(failure) = error {
            let stamped_error = stamp_error(span, failure, ErrorStamp::FULL);
            if let (Some((error_type, message)), Some(recorder)) =
                (stamped_error, &self.event_recorder)
                && role == SpanRole::LlmCall
            {
                recorder.record_operation_exception(&OperationException {
                    span_context: span.span_context(),
                    error_type: &error_type,
                    message: &message,
                    stack_trace: failure.stack_trace.as_deref(),
                    timestamp_ns: end_time.and_then(unix_nanos),
                });
            }
        }
        match end_time {
            Some(closed_at) => span.end_with_timestamp(closed_at),
            None => span.end(),
        }
    }

    // the room the error attributes will take is kept free, then indexed messages shed to fit the rest
    fn fit_to_budget(&self, mapped: AttributeMap, error: Option<&SpanError>) -> AttributeMap {
        let stamped = error.map_or_else(AttributeMap::new, error_attributes);
        let reserved = stamped
            .keys()
            .filter(|key| !mapped.contains_key(*key))
            .count();
        let budget = self
            .attribute_limit
            .map(|limit| limit.saturating_sub(reserved));
        fit_indexed_messages(mapped, budget)
    }

    fn mapped_attributes(&self, data: &SpanData<'_>) -> AttributeMap {
        let mut mapped = AttributeMap::new();
        for mapper in &self.mappers {
            mapped.extend(mapper.map(data, self.environment.as_ref()));
        }
        mapped
    }
}

fn unix_nanos(time: SystemTime) -> Option<u64> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    let nanos = u64::try_from(elapsed.as_nanos()).ok()?;
    Some(nanos)
}

// the two things every payload kind names: the call it belongs to and the failure it carries
struct SpanFacts<'data> {
    call_id: Option<&'data str>,
    error: Option<&'data SpanError>,
}

fn span_facts<'data>(data: &'data SpanData<'_>) -> SpanFacts<'data> {
    match data {
        SpanData::LLMCall(call) => SpanFacts {
            call_id: call.identity.call_id.as_deref(),
            error: call.error.as_ref(),
        },
        SpanData::MCPToolCall(call) => SpanFacts {
            call_id: call.identity.call_id.as_deref(),
            error: call.error.as_ref(),
        },
        SpanData::MCPListTools(list) => SpanFacts {
            call_id: list.identity.call_id.as_deref(),
            error: list.error.as_ref(),
        },
        SpanData::Guardrail(guardrail) => SpanFacts {
            call_id: None,
            error: guardrail.error.as_ref(),
        },
        SpanData::Service(service) => SpanFacts {
            call_id: None,
            error: service.error.as_ref(),
        },
    }
}

fn span_name(data: &SpanData<'_>) -> String {
    match data {
        SpanData::LLMCall(call) => NamedSpan::LlmCall {
            operation: call.operation,
            request_model: &call.request_model,
        }
        .name(),
        SpanData::MCPToolCall(call) => NamedSpan::McpToolCall {
            method: call.method,
            tool_name: &call.tool_name,
        }
        .name(),
        SpanData::MCPListTools(list) => NamedSpan::McpListTools {
            method: list.method,
        }
        .name(),
        SpanData::Guardrail(guardrail) => NamedSpan::Guardrail {
            guardrail_name: &guardrail.guardrail_name,
        }
        .name(),
        SpanData::Service(service) => NamedSpan::Service {
            service_name: &service.service_name,
            call_type: service.call_type.as_deref().unwrap_or(""),
        }
        .name(),
    }
}

#[path = "span_emission/tests.rs"]
mod tests;
