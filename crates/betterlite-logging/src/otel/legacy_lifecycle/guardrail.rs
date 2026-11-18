use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, TraceContextExt as _, Tracer as _};
use opentelemetry_sdk::trace::SdkTracer;

use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::dedupe::ScopeValue;
use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::error::LegacyLifecycleError;
use super::lifecycle::{self, LegacyLifecycle, standard_logging_object};
use super::state::scope_value;

pub const GUARDRAIL_SPAN_NAME: &str = "guardrail";

// LiteLLM's safe_set_attribute turns a missing value into "", never into an absent attribute
const MISSING_ATTRIBUTE_VALUE: OtelModelValue = OtelModelValue::Null(());

impl LegacyLifecycle {
    pub fn create_guardrail_span(
        &self,
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
        let tracer = self.tracer_for_request(kwargs)?;
        for entry in information.iter().filter_map(OtelModelValue::as_object) {
            self.emit_guardrail_entry(&tracer, kwargs, context, entry)?;
        }
        Ok(())
    }

    fn emit_guardrail_entry(
        &self,
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
        if !self.emit_once(kwargs, &scope) {
            return Ok(());
        }
        let start = guardrail_time(start_value, "guardrail_information.start_time")?;
        let end = guardrail_time(entry.get("end_time"), "guardrail_information.end_time")?;
        let builder = tracer
            .span_builder(GUARDRAIL_SPAN_NAME)
            .with_start_time(lifecycle::instant_from_ns(to_ns(start, self.now_ns())));
        let mut span = tracer.build_with_context(builder, context);
        write_guardrail_identity(&mut span, entry);
        write_guardrail_payload(&mut span, entry)?;
        self.write_team_attributes_from_std_log(&mut span, kwargs);
        span.end_with_timestamp(lifecycle::instant_from_ns(to_ns(end, self.now_ns())));
        Ok(())
    }

    pub fn emit_guardrail_spans_from_request_data(
        &self,
        request_data: &MetadataMap,
        parent: Option<&LiveSpan>,
    ) -> Result<(), LegacyLifecycleError> {
        let bucket = if request_data.contains_key("litellm_metadata") {
            "litellm_metadata"
        } else {
            "metadata"
        };
        let metadata = request_data
            .get(bucket)
            .and_then(OtelModelValue::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(information) = metadata.get("standard_logging_guardrail_information") else {
            return Ok(());
        };
        // the same bucket reaches the failure handler, so the entry markers share the request key
        let standard_logging_object = MetadataMap::from([
            ("guardrail_information".to_owned(), information.clone()),
            (
                "metadata".to_owned(),
                OtelModelValue::Object(metadata.clone()),
            ),
        ]);
        let litellm_params =
            MetadataMap::from([("metadata".to_owned(), OtelModelValue::Object(metadata))]);
        let kwargs = MetadataMap::from([
            (
                "litellm_params".to_owned(),
                OtelModelValue::Object(litellm_params),
            ),
            (
                "standard_logging_object".to_owned(),
                OtelModelValue::Object(standard_logging_object),
            ),
        ]);
        let context = parent.map_or_else(Context::new, |span| {
            Context::new().with_remote_span_context(span.context.clone())
        });
        self.create_guardrail_span(&kwargs, &context)
    }
}

fn write_guardrail_identity<S>(span: &mut S, entry: &MetadataMap)
where
    S: opentelemetry::trace::Span,
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
    S: opentelemetry::trace::Span,
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
    S: opentelemetry::trace::Span,
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
