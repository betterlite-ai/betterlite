use betterlite_logging::langfuse_observations::sampler::TraceIdHashSampler;
use betterlite_logging::langfuse_observations::span::parent_context;
use betterlite_models::decisions::Json;
use opentelemetry::trace::{SpanId, SpanKind, TraceContextExt as _, TraceId};
use opentelemetry_sdk::trace::{IdGenerator, SamplingDecision, ShouldSample as _};
use serde_json::json;

use super::values::{key_values, pairs, parse_span, parse_trace};
use crate::helpers::HelperError;

pub(crate) fn sampler_step(
    rate: f64,
    trace_id: &str,
    attributes: &[(String, Json)],
) -> Result<serde_json::Value, Box<HelperError>> {
    let sampled_trace = parse_trace(trace_id)?;
    let converted = key_values(attributes);
    let result = TraceIdHashSampler { rate }.should_sample(
        None,
        sampled_trace,
        "span",
        &SpanKind::Internal,
        &converted,
        &[],
    );
    let sampled = result.decision == SamplingDecision::RecordAndSample;
    Ok(json!({
        "decision": decision_name(&result.decision),
        "attributes": if sampled { pairs(&result.attributes) } else { serde_json::Value::Null },
    }))
}

const fn decision_name(decision: &SamplingDecision) -> &'static str {
    match decision {
        SamplingDecision::Drop => "drop",
        SamplingDecision::RecordOnly => "record_only",
        SamplingDecision::RecordAndSample => "record_and_sample",
    }
}

pub(crate) fn parent_step(
    trace_id: &str,
    parent_observation_id: Option<&str>,
    existing_trace: bool,
    fallback_span_id: &str,
) -> Result<serde_json::Value, Box<HelperError>> {
    let id_generator = FixedIdGenerator {
        span_id: parse_span(fallback_span_id)?,
    };
    let parent = parent_observation_id.map(parse_span).transpose()?;
    let context = parent_context(
        parse_trace(trace_id)?,
        parent,
        existing_trace,
        &id_generator,
    );
    let span_context = context.span().span_context().clone();
    Ok(json!({
        "valid": span_context.is_valid(),
        "trace_id": span_context.is_valid().then(|| span_context.trace_id().to_string()),
        "span_id": span_context.is_valid().then(|| span_context.span_id().to_string()),
        "is_remote": span_context.is_remote(),
        "sampled": span_context.is_sampled(),
    }))
}

#[derive(Clone, Copy, Debug)]
struct FixedIdGenerator {
    span_id: SpanId,
}

impl IdGenerator for FixedIdGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::INVALID
    }

    fn new_span_id(&self) -> SpanId {
        self.span_id
    }
}
