use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::trace::{
    Span, SpanBuilder, SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
    Tracer,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::IdGenerator;

use super::attributes::keys::TRACE_PUBLIC;
use super::identity::{Timestamp, to_unix_nanos};
use super::requested_ids::RequestedIds;

// litellm.integrations.langfuse.langfuse_sdk.LangfuseObservation; public rides along for child observations
pub struct LangfuseObservation<S> {
    pub id: String,
    pub span: S,
    pub public: Option<bool>,
}

impl<S: Span> LangfuseObservation<S> {
    // the resolved id comes from the span context, so it always names the span that was started
    #[must_use]
    pub fn new(span: S, public: Option<bool>) -> Self {
        Self {
            id: span.span_context().span_id().to_string(),
            span,
            public,
        }
    }

    pub fn end(&mut self, end_time: Option<Timestamp>) {
        match to_unix_nanos(end_time).and_then(system_time) {
            Some(timestamp) => self.span.end_with_timestamp(timestamp),
            None => self.span.end(),
        }
    }
}

// litellm.integrations.langfuse.langfuse_sdk.start_generation, with the tracer and id generator injected
pub struct GenerationStart {
    pub trace_id: TraceId,
    pub parent_observation_id: Option<SpanId>,
    pub existing_trace: bool,
    pub observation_id: Option<SpanId>,
    pub name: String,
    pub start_time: Option<Timestamp>,
    pub public: Option<bool>,
    pub attributes: Vec<KeyValue>,
}

// litellm.integrations.langfuse.langfuse_sdk.start_child_span; the child repeats the generation's public flag
pub struct ChildSpanStart {
    pub name: String,
    pub start_time: Option<Timestamp>,
    pub attributes: Vec<KeyValue>,
}

// litellm's _parent_context: a fresh trace hangs nowhere; a continuation hangs under a synthetic remote parent
#[must_use]
pub fn parent_context(
    trace_id: TraceId,
    parent_observation_id: Option<SpanId>,
    existing_trace: bool,
    id_generator: &dyn IdGenerator,
) -> Context {
    if parent_observation_id.is_none() && !existing_trace {
        return Context::new();
    }
    let parent_span_id = parent_observation_id.unwrap_or_else(|| id_generator.new_span_id());
    let parent = SpanContext::new(
        trace_id,
        parent_span_id,
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    );
    Context::new().with_remote_span_context(parent)
}

pub fn start_generation<T>(
    tracer: &T,
    id_generator: &dyn IdGenerator,
    start: GenerationStart,
) -> LangfuseObservation<T::Span>
where
    T: Tracer,
{
    let context = parent_context(
        start.trace_id,
        start.parent_observation_id,
        start.existing_trace,
        id_generator,
    );
    let span = {
        let _requested = RequestedIds::enter(Some(start.trace_id), start.observation_id);
        tracer.build_with_context(
            span_builder(start.name, start.start_time, start.attributes),
            &context,
        )
    };
    LangfuseObservation::new(span, start.public)
}

pub fn start_child_span<T>(
    tracer: &T,
    parent: &LangfuseObservation<T::Span>,
    start: ChildSpanStart,
) -> LangfuseObservation<T::Span>
where
    T: Tracer,
{
    let context = Context::new().with_remote_span_context(parent.span.span_context().clone());
    let mut attributes = Vec::new();
    if let Some(public) = parent.public {
        attributes.push(KeyValue::new(TRACE_PUBLIC, public));
    }
    attributes.extend(start.attributes);
    let span = {
        let _requested = RequestedIds::enter(None, None);
        tracer.build_with_context(
            span_builder(start.name, start.start_time, attributes),
            &context,
        )
    };
    LangfuseObservation::new(span, parent.public)
}

fn span_builder(
    name: String,
    start_time: Option<Timestamp>,
    attributes: Vec<KeyValue>,
) -> SpanBuilder {
    let mut builder = SpanBuilder::from_name(name).with_attributes(attributes);
    if let Some(timestamp) = to_unix_nanos(start_time).and_then(system_time) {
        builder = builder.with_start_time(timestamp);
    }
    builder
}

fn system_time(nanos: i64) -> Option<SystemTime> {
    let magnitude = Duration::from_nanos(nanos.unsigned_abs());
    if nanos < 0 {
        UNIX_EPOCH.checked_sub(magnitude)
    } else {
        UNIX_EPOCH.checked_add(magnitude)
    }
}

#[path = "span/tests.rs"]
mod tests;
