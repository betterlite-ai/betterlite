use betterlite_logging::langfuse_observations::sampler::TraceIdHashSampler;
use betterlite_logging::langfuse_observations::span::parent_context;
use opentelemetry::trace::{SpanId, SpanKind, TraceContextExt as _, TraceId};
use opentelemetry_sdk::trace::{IdGenerator, SamplingDecision, ShouldSample as _};
use serde_json::json;

use super::inputs::{HexSpanId, HexTraceId};
use super::values::{context_parts, key_values, pairs};

pub(crate) fn sampler_step(
    rate: f64,
    trace_id: &HexTraceId,
    attributes: &[(String, String)],
) -> serde_json::Value {
    let result = TraceIdHashSampler { rate }.should_sample(
        None,
        trace_id.0,
        "span",
        &SpanKind::Internal,
        &key_values(attributes),
        &[],
    );
    let (decision, kept) = match result.decision {
        SamplingDecision::Drop => ("drop", serde_json::Value::Null),
        SamplingDecision::RecordOnly => ("record_only", pairs(&result.attributes)),
        SamplingDecision::RecordAndSample => ("record_and_sample", pairs(&result.attributes)),
    };
    json!({"decision": decision, "attributes": kept})
}

pub(crate) fn parent_step(
    trace_id: &HexTraceId,
    parent_observation_id: Option<&HexSpanId>,
    existing_trace: bool,
    fallback_span_id: &HexSpanId,
) -> serde_json::Value {
    let context = parent_context(
        trace_id.0,
        parent_observation_id.map(|parent| parent.0),
        existing_trace,
        &FixedIdGenerator(fallback_span_id.0),
    );
    context_parts(context.span().span_context())
}

#[derive(Debug)]
struct FixedIdGenerator(SpanId);

impl IdGenerator for FixedIdGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::INVALID
    }

    fn new_span_id(&self) -> SpanId {
        self.0
    }
}
