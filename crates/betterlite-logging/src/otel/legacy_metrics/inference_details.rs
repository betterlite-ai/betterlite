use opentelemetry::logs::AnyValue;

use super::INFERENCE_DETAILS_EVENT;
use super::error::LegacyMetricsError;
use super::events::{EventAttributes, EventCall, SemanticEventEmitter, choices_of};
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::semconv_attrs::gen_ai_operation_name;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::transforms::{transform_choices, transform_messages};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

mod tests;

pub fn build_attributes(
    call: &EventCall<'_>,
    provider: Option<&str>,
) -> Result<EventAttributes, LegacyMetricsError> {
    let call_type = call
        .kwargs
        .get("call_type")
        .and_then(OtelModelValue::as_str);
    let mut attributes: EventAttributes =
        vec![("event_name", AnyValue::from(INFERENCE_DETAILS_EVENT))];
    if let Some(name) = provider {
        attributes.push(("gen_ai.provider.name", AnyValue::from(name.to_owned())));
    }
    attributes.push((
        "gen_ai.operation.name",
        AnyValue::from(gen_ai_operation_name(call_type)),
    ));
    if !call.capture_mode.capture_in_event() {
        return Ok(attributes);
    }
    let messages = call
        .kwargs
        .get("messages")
        .filter(|messages| is_truthy(messages))
        .map(transform_messages)
        .unwrap_or_default();
    let choices = transform_choices(choices_of(call.response));
    for (key, transformed) in [
        ("gen_ai.input.messages", messages),
        ("gen_ai.output.messages", choices),
    ] {
        if !transformed.is_empty() {
            let text = safe_dumps(&transformed, DEFAULT_MAX_RECURSE_DEPTH, None)
                .map_err(|source| LegacyMetricsError::Dump { source })?;
            attributes.push((key, AnyValue::from(text)));
        }
    }
    Ok(attributes)
}

pub fn emit_inference_details(
    emitter: &SemanticEventEmitter,
    call: &EventCall<'_>,
    provider: Option<&str>,
) -> Result<(), LegacyMetricsError> {
    let attributes = build_attributes(call, provider)?;
    emitter.emit_record(call.span_context, None, attributes);
    Ok(())
}
