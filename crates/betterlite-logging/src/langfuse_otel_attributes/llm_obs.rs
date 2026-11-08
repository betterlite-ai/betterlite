use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Span;

use crate::langfuse_projections::content::ResponseObj;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::{LLMObsAttributes, SafeSetAttribute as _};

use super::keys::LangfuseSpanAttributes;
use super::output_content::get_output_content_by_type;
use super::wire::json_value;

// litellm.integrations.langfuse.langfuse_otel_attributes.LangfuseLLMObsOTELAttributes
pub struct LangfuseLLMObsOTELAttributes;

impl LLMObsAttributes for LangfuseLLMObsOTELAttributes {
    fn set_messages<S>(span: &mut S, kwargs: &MetadataMap)
    where
        S: Span,
    {
        let mut prompt = IndexMap::new();
        prompt.insert(
            "messages".to_owned(),
            kwargs
                .get("messages")
                .cloned()
                .unwrap_or(OtelModelValue::Null(())),
        );
        let optional = kwargs
            .get("optional_params")
            .and_then(OtelModelValue::as_object);
        for key in ["functions", "tools"] {
            if let Some(value) = optional
                .and_then(|fields| fields.get(key))
                .filter(|value| !matches!(value, OtelModelValue::Null(())))
            {
                prompt.insert(key.to_owned(), value.clone());
            }
        }
        if let Ok(record) = serde_json::to_string(&prompt) {
            span.safe_set_attribute(
                LangfuseSpanAttributes::ObservationInput.value(),
                &OtelModelValue::String(record),
            );
        }
    }

    fn set_response_output_messages<S>(span: &mut S, response: &OtelModelValue)
    where
        S: Span,
    {
        let json = json_value(response);
        let response_obj = match &json {
            Json::Object(_) => Some(ResponseObj::Dict(&json)),
            Json::List(_) => Some(ResponseObj::List(&json)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
        };
        let content = get_output_content_by_type(response_obj.as_ref(), None);
        span.safe_set_attribute(
            LangfuseSpanAttributes::ObservationOutput.value(),
            &OtelModelValue::String(content),
        );
    }
}
