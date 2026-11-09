use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::langfuse_observations::requested_ids::requested_ids;
use betterlite_logging::langfuse_observations::span::{
    ChildSpanStart, LangfuseObservation, start_child_span,
};
use opentelemetry::trace::{
    Span, SpanBuilder, SpanContext, SpanId, Status, TraceContextExt as _, TraceFlags, TraceId,
    TraceState, Tracer,
};
use opentelemetry::{Context, KeyValue};
use serde_json::json;

use super::inputs::ChildSpan;
use super::values::{context_parts, key_values, pairs};

#[derive(Default)]
struct Recorded {
    name: String,
    start_time: Option<SystemTime>,
    attributes: Vec<KeyValue>,
    parent: Option<SpanContext>,
    requested_trace_id: Option<TraceId>,
    requested_span_id: Option<SpanId>,
}

struct ScriptedSpan {
    context: SpanContext,
    recorded: Recorded,
}

impl Span for ScriptedSpan {
    fn add_event_with_timestamp<T>(&mut self, _name: T, _time: SystemTime, _attrs: Vec<KeyValue>)
    where
        T: Into<Cow<'static, str>>,
    {
    }
    fn span_context(&self) -> &SpanContext {
        &self.context
    }
    fn is_recording(&self) -> bool {
        false
    }
    fn set_attribute(&mut self, _attribute: KeyValue) {}
    fn set_status(&mut self, _status: Status) {}
    fn update_name<T>(&mut self, _new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }
    fn add_link(&mut self, _span_context: SpanContext, _attributes: Vec<KeyValue>) {}
    fn end_with_timestamp(&mut self, _timestamp: SystemTime) {}
}

struct ScriptedTracer {
    span_id: SpanId,
}

impl Tracer for ScriptedTracer {
    type Span = ScriptedSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> ScriptedSpan {
        let (requested_trace_id, requested_span_id) = requested_ids();
        ScriptedSpan {
            context: SpanContext::new(
                TraceId::INVALID,
                self.span_id,
                TraceFlags::SAMPLED,
                false,
                TraceState::NONE,
            ),
            recorded: Recorded {
                name: builder.name.to_string(),
                start_time: builder.start_time,
                attributes: builder.attributes.unwrap_or_default(),
                parent: Some(parent_cx.span().span_context().clone()),
                requested_trace_id,
                requested_span_id,
            },
        }
    }
}

pub(crate) fn child_step(step: &ChildSpan) -> serde_json::Value {
    let parent = LangfuseObservation::new(
        ScriptedSpan {
            context: SpanContext::new(
                step.parent_trace_id.0,
                step.parent_span_id.0,
                TraceFlags::SAMPLED,
                false,
                TraceState::NONE,
            ),
            recorded: Recorded::default(),
        },
        step.parent_public,
    );
    let tracer = ScriptedTracer {
        span_id: step.span_id.0,
    };
    let child = start_child_span(
        &tracer,
        &parent,
        ChildSpanStart {
            name: step.name.clone(),
            start_time: step.moment.timestamp(),
            attributes: key_values(&step.attributes),
        },
    );
    recorded_projection(&child.span.recorded, &child.id, child.public)
}

fn recorded_projection(
    recorded: &Recorded,
    observation_id: &str,
    public: Option<bool>,
) -> serde_json::Value {
    let start_nanos = recorded
        .start_time
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok());
    json!({
        "requested_trace_id": recorded.requested_trace_id.map(|id| id.to_string()),
        "requested_span_id": recorded.requested_span_id.map(|id| id.to_string()),
        "name": recorded.name,
        "start_time": start_nanos,
        "attributes": pairs(&recorded.attributes),
        "parent": context_parts(recorded.parent.as_ref().unwrap_or(&SpanContext::NONE)),
        "observation_id": observation_id,
        "public": public,
    })
}
