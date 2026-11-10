use opentelemetry::trace::Span;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;

use super::request::UNKNOWN_PROVIDER;

mod tests;

const ATTRIBUTE_PREFIX: &str = "llm.";
const STRINGIFIED_RAW_RESPONSE: &str = "stringified_raw_response";

pub fn set_raw_request_attributes<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    let empty = MetadataMap::new();
    let provider = provider_label(
        kwargs
            .get("litellm_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty),
    );
    let complete_input = kwargs
        .get("additional_args")
        .and_then(OtelModelValue::as_object)
        .and_then(|args| args.get("complete_input_dict"))
        .and_then(OtelModelValue::as_object)
        .filter(|entries| !entries.is_empty());
    if let Some(entries) = complete_input {
        write_entries(span, &provider, entries);
    }
    let Some(text) = kwargs
        .get("original_response")
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    match parse_raw_response(text) {
        Some(OtelModelValue::Object(entries)) => write_entries(span, &provider, &entries),
        Some(_) => tracing::error!("OpenTelemetry logging error in set_raw_request_attributes"),
        None => {
            tracing::debug!("raw_response not json string");
            span.safe_set_attribute(
                &format!("{ATTRIBUTE_PREFIX}{provider}.{STRINGIFIED_RAW_RESPONSE}"),
                &OtelModelValue::String(text.to_owned()),
            );
        }
    }
}

fn provider_label(litellm_params: &MetadataMap) -> String {
    match litellm_params.get("custom_llm_provider") {
        None => UNKNOWN_PROVIDER.to_owned(),
        Some(OtelModelValue::Null(())) => "None".to_owned(),
        Some(OtelModelValue::String(text)) => text.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn write_entries<S>(span: &mut S, provider: &str, entries: &MetadataMap)
where
    S: Span,
{
    for (param, value) in entries {
        span.safe_set_attribute(&format!("{ATTRIBUTE_PREFIX}{provider}.{param}"), value);
    }
}

#[must_use]
pub fn parse_raw_response(text: &str) -> Option<OtelModelValue> {
    let Ok(value) = serde_json::from_str(text) else {
        return None;
    };
    Some(value)
}
