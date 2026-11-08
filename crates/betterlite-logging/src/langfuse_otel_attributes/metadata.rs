use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Span;

use crate::langfuse_projections::metadata_header::add_metadata_from_header;
use crate::otel::span_attributes::SafeSetAttribute as _;

use super::keys::LangfuseSpanAttributes;
use super::wire::{field, model_value, object};

const METADATA_ATTRIBUTES: [(&str, LangfuseSpanAttributes); 15] = [
    ("generation_name", LangfuseSpanAttributes::GenerationName),
    ("generation_id", LangfuseSpanAttributes::GenerationId),
    (
        "parent_observation_id",
        LangfuseSpanAttributes::ParentObservationId,
    ),
    ("mask_input", LangfuseSpanAttributes::MaskInput),
    ("mask_output", LangfuseSpanAttributes::MaskOutput),
    ("trace_user_id", LangfuseSpanAttributes::TraceUserId),
    ("session_id", LangfuseSpanAttributes::SessionId),
    ("tags", LangfuseSpanAttributes::Tags),
    ("trace_name", LangfuseSpanAttributes::TraceName),
    ("trace_id", LangfuseSpanAttributes::TraceId),
    ("trace_metadata", LangfuseSpanAttributes::TraceMetadata),
    ("trace_release", LangfuseSpanAttributes::Release),
    ("existing_trace_id", LangfuseSpanAttributes::ExistingTraceId),
    ("update_trace_keys", LangfuseSpanAttributes::UpdateTraceKeys),
    ("debug_langfuse", LangfuseSpanAttributes::DebugLangfuse),
];

// litellm.integrations.langfuse.langfuse_otel.LangfuseOtelLogger._extract_langfuse_metadata
pub(crate) fn extract_langfuse_metadata(kwargs: &Json) -> IndexMap<String, Json> {
    let litellm_params = field(kwargs, "litellm_params");
    let metadata = litellm_params
        .and_then(object)
        .and_then(|params| params.get("metadata"))
        .and_then(object)
        .cloned()
        .unwrap_or_default();
    add_metadata_from_header(litellm_params, metadata)
}

// litellm.integrations.langfuse.langfuse_otel.LangfuseOtelLogger._set_metadata_attributes
pub(crate) fn set_metadata_attributes<S>(span: &mut S, metadata: &IndexMap<String, Json>)
where
    S: Span,
{
    let version = metadata
        .get("trace_version")
        .filter(|value| !matches!(value, Json::Null))
        .or_else(|| metadata.get("version"))
        .filter(|value| !matches!(value, Json::Null));
    if let Some(version_value) = version {
        span.safe_set_attribute(
            LangfuseSpanAttributes::Version.value(),
            &model_value(version_value),
        );
    }
    for (key, attribute) in METADATA_ATTRIBUTES {
        let Some(value) = metadata
            .get(key)
            .filter(|value| !matches!(value, Json::Null))
        else {
            continue;
        };
        let stripped = match value {
            Json::Text(text) if key == "trace_id" => Json::Text(text.replace('-', "")),
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => value.clone(),
        };
        let written = match &stripped {
            Json::List(_) | Json::Object(_) => {
                serde_json::to_string(&stripped).map_or_else(|_source| stripped.clone(), Json::Text)
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => stripped.clone(),
        };
        span.safe_set_attribute(attribute.value(), &model_value(&written));
    }
}
