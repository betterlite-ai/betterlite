use betterlite_logging::langfuse_observations::identity::Timestamp;
use betterlite_logging::langfuse_observations::span::{
    ChildSpanStart, LangfuseObservation, start_child_span,
};
use opentelemetry::KeyValue;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use serde_json::json;

use super::inputs::Step;
use super::projections::timestamp;
use super::scripted::{ScriptedSpan, ScriptedTracer, system_nanos};
use super::values::{invalid, key_values, pairs, parse_span, parse_trace};
use crate::helpers::HelperError;

pub(crate) fn child_step(step: &Step) -> Result<serde_json::Value, Box<HelperError>> {
    let Step::StartChildSpan {
        parent_trace_id,
        parent_span_id,
        parent_public,
        name,
        seconds,
        datetime,
        attributes,
        child_span_id,
    } = step
    else {
        return Err(invalid("not a child span step"));
    };
    let parameters = child_parameters(
        ParentSpec {
            trace_id: parse_trace(parent_trace_id)?,
            span_id: parse_span(parent_span_id)?,
            public: *parent_public,
        },
        name.clone(),
        timestamp(*seconds, datetime.as_deref())?,
        key_values(attributes),
    );
    Ok(child(parameters, parse_span(child_span_id)?))
}

#[derive(Clone, Copy, Debug)]
struct ParentSpec {
    trace_id: TraceId,
    span_id: SpanId,
    public: Option<bool>,
}

struct ChildParameters {
    parent: LangfuseObservation<ScriptedSpan>,
    start: ChildSpanStart,
}

fn child_parameters(
    parent_spec: ParentSpec,
    name: String,
    start_time: Option<Timestamp>,
    attributes: Vec<KeyValue>,
) -> ChildParameters {
    let context = SpanContext::new(
        parent_spec.trace_id,
        parent_spec.span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );
    ChildParameters {
        parent: LangfuseObservation::new(
            ScriptedSpan {
                context,
                parent: SpanContext::NONE,
                name: String::new(),
                start_time: None,
                attributes: Vec::new(),
                requested_trace_id: None,
                requested_span_id: None,
                end_time: None,
            },
            parent_spec.public,
        ),
        start: ChildSpanStart {
            name,
            start_time,
            attributes,
        },
    }
}

fn child(parameters: ChildParameters, span_id: SpanId) -> serde_json::Value {
    let tracer = ScriptedTracer { span_id };
    let observation = start_child_span(&tracer, &parameters.parent, parameters.start);
    recorded(&observation)
}

fn recorded(observation: &LangfuseObservation<ScriptedSpan>) -> serde_json::Value {
    let span = &observation.span;
    json!({
        "requested_trace_id": span.requested_trace_id.map(|id| id.to_string()),
        "requested_span_id": span.requested_span_id.map(|id| id.to_string()),
        "name": span.name,
        "start_time": span.start_time.and_then(system_nanos),
        "attributes": pairs(&span.attributes),
        "parent_valid": span.parent.is_valid(),
        "parent_trace_id": span.parent.is_valid().then(|| span.parent.trace_id().to_string()),
        "parent_span_id": span.parent.is_valid().then(|| span.parent.span_id().to_string()),
        "parent_is_remote": span.parent.is_remote(),
        "observation_id": observation.id,
        "public": observation.public,
    })
}
