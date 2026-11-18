use crate::otel::model::metadata::wire::MetadataError;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::metadata::wire::callback::Callback;
use crate::otel::model::metadata::wire::decode;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::{
    PARENT_SPAN_KEY, ParentSpan, span_context_from_kwargs,
};

// LiteLLM's _get_span_context reads the whole kwargs dict, which the typed Callback reader takes as JSON
pub fn from_kwargs(kwargs: &MetadataMap) -> Result<ParentSpan, MetadataError> {
    let text = serde_json::to_string(kwargs)?;
    let callback = decode::<Callback>(&text)?;
    Ok(span_context_from_kwargs(&callback))
}

// _end_proxy_span_from_kwargs reads the provider-safe bucket first and the proxy bucket second
#[must_use]
pub fn proxy_carrier(kwargs: &MetadataMap, with_litellm_metadata: bool) -> Option<&OtelModelValue> {
    let params = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object);
    let direct = params
        .and_then(|fields| fields.get("metadata"))
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get(PARENT_SPAN_KEY))
        .filter(|value| !matches!(value, OtelModelValue::Null(())));
    if direct.is_some() || !with_litellm_metadata {
        return direct;
    }
    params?
        .get("litellm_metadata")?
        .as_object()?
        .get(PARENT_SPAN_KEY)
}
