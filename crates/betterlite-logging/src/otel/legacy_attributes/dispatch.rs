use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use opentelemetry::trace::Span;

use crate::langfuse_otel_attributes::llm_obs::LangfuseLLMObsOTELAttributes;
use crate::langfuse_otel_attributes::writer::set_langfuse_otel_attributes;
use crate::otel::integrations::arize_attributes::set_attributes as set_arize_attributes;
use crate::otel::integrations::langtrace::LangtraceAttributes;
use crate::otel::integrations::weave_attributes::set_weave_otel_attributes;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::spend_bodies::redaction::should_redact_message_logging;
use crate::spend_bodies::support::Fields;

use super::content::set_content_attributes;
use super::context::{AttributeContext, InferenceCall};
use super::error::LegacyAttributesError;
use super::request::set_request_attributes;
use super::response::set_response_attributes;
use last_wins::LastWins;

mod last_wins;
mod tests;

const VENDOR_NAMES: [&str; 3] = ["langtrace", "langfuse_otel", "weave_otel"];

pub fn set_attributes<S>(
    span: &mut S,
    context: &AttributeContext<'_>,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
) -> Result<(), LegacyAttributesError>
where
    S: Span,
{
    let vendor = context
        .callback_name
        .filter(|name| VENDOR_NAMES.contains(name));
    if let Some(name) = vendor {
        LastWins::write(span, |buffered| {
            write_vendor_attributes(buffered, name, kwargs, response);
        });
        return Ok(());
    }
    let empty = MetadataMap::new();
    let payload = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
        .ok_or(LegacyAttributesError::MissingStandardLoggingObject)?;
    let call = InferenceCall {
        kwargs,
        optional_params: kwargs
            .get("optional_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty),
        litellm_params: kwargs
            .get("litellm_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty),
        payload,
        response,
    };
    set_request_attributes(span, context, &call);
    if !context.capture_mode.capture_in_span() {
        return Ok(());
    }
    set_content_attributes(span, context, &call);
    set_response_attributes(span, response);
    Ok(())
}

fn write_vendor_attributes<S>(
    span: &mut S,
    name: &str,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
) where
    S: Span,
{
    match name {
        "langtrace" => match response.and_then(OtelModelValue::as_object) {
            Some(response_obj) => {
                LangtraceAttributes::set_langtrace_attributes(span, kwargs, response_obj);
            }
            // LiteLLM raises on None.get("id") after the request attributes, so no completion event
            None => write_langtrace_request(span, kwargs),
        },
        "langfuse_otel" => {
            // the legacy logger carries no environment snapshot; only the dynamic param applies
            let environment: BTreeMap<String, String> = BTreeMap::new();
            let redact = redact_messages(kwargs);
            // LiteLLM runs the shared Arize attribute writer first, with the Langfuse message layout
            set_arize_attributes::<LangfuseLLMObsOTELAttributes, S>(
                span,
                kwargs,
                response.unwrap_or(&OtelModelValue::Null(())),
                redact,
            );
            set_langfuse_otel_attributes(span, kwargs, response, &environment, redact);
        }
        "weave_otel" => {
            set_weave_otel_attributes(
                span,
                kwargs,
                response.unwrap_or(&OtelModelValue::Null(())),
                redact_messages(kwargs),
            );
        }
        _ => {}
    }
}

fn write_langtrace_request<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    let mut options = kwargs.clone();
    if let Some(optional) = kwargs
        .get("optional_params")
        .and_then(OtelModelValue::as_object)
    {
        options.extend(
            optional
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    let vendor = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("custom_llm_provider"))
        .and_then(OtelModelValue::as_str);
    LangtraceAttributes::set_request_attributes(span, &options, vendor);
}

fn redact_messages(kwargs: &MetadataMap) -> bool {
    let litellm_params = json_fields(kwargs.get("litellm_params"));
    let dynamic_params = json_fields(kwargs.get("standard_callback_dynamic_params"));
    should_redact_message_logging(litellm_params.as_ref(), dynamic_params.as_ref(), false)
}

fn json_fields(value: Option<&OtelModelValue>) -> Option<Fields> {
    let encoded = serde_json::to_value(value?).ok()?;
    let decoded: Json = serde_json::from_value(encoded).ok()?;
    match decoded {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
