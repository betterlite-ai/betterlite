use crate::datadog::apm::{TraceContext, Tracer};

use super::types::DatadogPayload;

// LiteLLM sets dd.trace_id whenever an active span has one and dd.span_id only when the span has it too
pub fn add_trace_context(payload: &mut DatadogPayload, tracer: &dyn Tracer) {
    let Some(context) = TraceContext::active(tracer) else {
        return;
    };
    payload.trace_id = Some(context.trace_id);
    payload.span_id = context.span_id;
}
