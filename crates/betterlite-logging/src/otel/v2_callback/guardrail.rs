use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::mappers::base::SpanData;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::vocabulary::registry::SpanRole;

// litellm.integrations.otel.logger.OpenTelemetryV2.emit_guardrail_span
pub fn emit_guardrail_span(
    callback: &V2Callback,
    entry: &BTreeMap<String, OtelModelValue>,
    trace: &RequestTraceContext,
) {
    let data = GuardrailSpanData::from_logging_entry(entry);
    let parent = trace.resolve_request_span_context();
    let options = EmitOptions {
        parent: Some(&parent),
        start_time: epoch_time(data.start_time),
        end_time: epoch_time(data.end_time),
        tracer: None,
        links: &[],
    };
    let _emitted =
        callback
            .logger
            .emitter
            .emit(SpanRole::Guardrail, &SpanData::Guardrail(&data), &options);
}

// the entry's epoch seconds as the instant LiteLLM's to_ns hands the emitter
fn epoch_time(seconds: Option<f64>) -> Option<SystemTime> {
    let value = seconds?;
    let elapsed = Duration::try_from_secs_f64(value).ok()?;
    UNIX_EPOCH.checked_add(elapsed)
}
