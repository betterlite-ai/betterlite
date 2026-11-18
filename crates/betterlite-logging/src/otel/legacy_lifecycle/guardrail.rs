use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span, Tracer as _};
use opentelemetry_sdk::trace::SdkTracer;

use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::dedupe::ScopeValue;
use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::context::standard_logging_object;
use super::error::LegacyLifecycleError;
use super::lifecycle::LegacyLifecycle;
use super::state::{request_key, scope_value};
use super::{context, recording, tracer};

pub const GUARDRAIL_SPAN_NAME: &str = "guardrail";

// LiteLLM's safe_set_attribute turns a missing value into "", never into an absent attribute
const MISSING_ATTRIBUTE_VALUE: OtelModelValue = OtelModelValue::Null(());

fn write_guardrail_identity<S>(span: &mut S, entry: &MetadataMap)
where
    S: Span,
{
    let kind = OtelModelValue::String("GUARDRAIL".to_owned());
    span.safe_set_attribute("openinference.span.kind", &kind);
    for attribute in ["guardrail_name", "guardrail_mode", "guardrail_status"] {
        let value = entry.get(attribute).unwrap_or(&MISSING_ATTRIBUTE_VALUE);
        span.safe_set_attribute(attribute, value);
    }
    if let Some(value) = entry.get("guardrail_action")
        && is_truthy(value)
    {
        span.safe_set_attribute("guardrail_action", value);
    }
}

fn write_guardrail_payload<S>(span: &mut S, entry: &MetadataMap) -> Result<(), LegacyLifecycleError>
where
    S: Span,
{
    for attribute in [
        "masked_entity_count",
        "guardrail_response",
        "guardrail_usage",
    ] {
        if let Some(value) = entry.get(attribute)
            && !matches!(value, OtelModelValue::Null(()))
        {
            write_dumped(span, attribute, value)?;
        }
    }
    if let Some(value) = entry.get("violation_categories")
        && is_truthy(value)
    {
        write_dumped(span, "guardrail_violation_categories", value)?;
    }
    if let Some(value) = entry.get("guardrail_cost")
        && !matches!(value, OtelModelValue::Null(()))
    {
        span.safe_set_attribute("guardrail_cost", value);
    }
    if let Some(flag) = entry
        .get("guardrail_cost_in_spend")
        .and_then(OtelModelValue::as_bool)
    {
        span.safe_set_attribute("guardrail_cost_in_spend", &OtelModelValue::Bool(flag));
    }
    Ok(())
}

fn write_dumped<S>(
    span: &mut S,
    attribute: &'static str,
    value: &OtelModelValue,
) -> Result<(), LegacyLifecycleError>
where
    S: Span,
{
    let text = safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None)
        .map_err(|source| LegacyLifecycleError::GuardrailJson { attribute, source })?;
    span.set_attribute(KeyValue::new(attribute, text));
    Ok(())
}

fn guardrail_time(
    value: Option<&OtelModelValue>,
    field: &'static str,
) -> Result<SpanTime, LegacyLifecycleError> {
    match value {
        None | Some(OtelModelValue::Null(())) => Ok(SpanTime::Missing),
        Some(OtelModelValue::Number(number)) => {
            number
                .as_f64()
                .map(SpanTime::Seconds)
                .ok_or(LegacyLifecycleError::GuardrailField {
                    field,
                    expected: "epoch seconds",
                })
        }
        // datetime.fromtimestamp reads Python's bool as the integer it subclasses
        Some(OtelModelValue::Bool(flag)) => Ok(SpanTime::Seconds(if *flag { 1.0 } else { 0.0 })),
        Some(_) => Err(LegacyLifecycleError::GuardrailField {
            field,
            expected: "epoch seconds",
        }),
    }
}

pub(super) fn create_guardrail_span(
    lifecycle: &LegacyLifecycle,
    kwargs: &MetadataMap,
    context: &Context,
) -> Result<(), LegacyLifecycleError> {
    let Some(payload) = standard_logging_object(kwargs) else {
        return Ok(());
    };
    let Some(information) = payload
        .get("guardrail_information")
        .and_then(OtelModelValue::as_array)
    else {
        return Ok(());
    };
    if !information.iter().any(|entry| entry.as_object().is_some()) {
        return Ok(());
    }
    let tracer = tracer::tracer_for_request(lifecycle, kwargs)?;
    for entry in information.iter().filter_map(OtelModelValue::as_object) {
        emit_guardrail_entry(lifecycle, &tracer, kwargs, context, entry)?;
    }
    Ok(())
}

pub(super) fn emit_guardrail_entry(
    lifecycle: &LegacyLifecycle,
    tracer: &SdkTracer,
    kwargs: &MetadataMap,
    context: &Context,
    entry: &MetadataMap,
) -> Result<(), LegacyLifecycleError> {
    let start_value = entry.get("start_time");
    // every lifecycle point re-reads the same entry list, so the invocation's identity dedupes
    let scope = [
        ScopeValue::Text(GUARDRAIL_SPAN_NAME.to_owned()),
        scope_value(entry.get("guardrail_name")),
        scope_value(start_value),
        scope_value(entry.get("guardrail_mode")),
    ];
    if !lifecycle
        .dedupe
        .emit_once(request_key(kwargs).as_deref(), &scope)
    {
        return Ok(());
    }
    let start = guardrail_time(start_value, "guardrail_information.start_time")?;
    let end = guardrail_time(entry.get("end_time"), "guardrail_information.end_time")?;
    let builder = tracer
        .span_builder(GUARDRAIL_SPAN_NAME)
        .with_start_time(context::instant_from_ns(to_ns(start, lifecycle.now_ns())));
    let mut span = tracer.build_with_context(builder, context);
    write_guardrail_identity(&mut span, entry);
    write_guardrail_payload(&mut span, entry)?;
    recording::write_team_attributes_from_log(&mut span, kwargs);
    span.end_with_timestamp(context::instant_from_ns(to_ns(end, lifecycle.now_ns())));
    Ok(())
}
