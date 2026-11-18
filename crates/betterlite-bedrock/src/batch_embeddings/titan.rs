use betterlite_models::decisions::Json as ModelJson;

use crate::bedrock_model_capabilities::ModelInfoLookup;

const INVOCATION_SCHEMA_FIELD: &str = "bedrock_invocation_schema";
const TITAN_V2_INVOCATION_SCHEMA: &str = "titan_v2";
const TITAN_V2_EMBED_MODEL_MARKER: &str = "titan-embed-text-v2";

// The registry's provider_specific_entry.<field>, kept only for a nonempty string, so every failure reads as None
pub async fn lookup_provider_specific_field<Lookup>(
    model: &str,
    field: &str,
    lookup: &Lookup,
) -> Option<String>
where
    Lookup: ModelInfoLookup,
{
    let info = lookup.bedrock_model_info(model).await.ok()?;
    let Some(ModelJson::Object(provider_specific)) = info.fields.get("provider_specific_entry")
    else {
        return None;
    };
    match provider_specific.get(field) {
        Some(ModelJson::Text(value)) if !value.is_empty() => Some(value.clone()),
        None
        | Some(
            ModelJson::Null
            | ModelJson::Bool(_)
            | ModelJson::Number(_)
            | ModelJson::Text(_)
            | ModelJson::List(_)
            | ModelJson::Object(_),
        ) => None,
    }
}

// A resolved id trusts its schema and gets no name fallback; only an unmapped or empty one reaches the marker
pub async fn is_titan_v2_embed_model<Lookup>(model: &str, lookup: &Lookup) -> bool
where
    Lookup: ModelInfoLookup,
{
    if let Some(schema) =
        lookup_provider_specific_field(model, INVOCATION_SCHEMA_FIELD, lookup).await
    {
        return schema == TITAN_V2_INVOCATION_SCHEMA;
    }
    has_titan_v2_marker(model)
}

// The first marker occurrence decides: only the end of the id or a ':' or '/' may follow it
fn has_titan_v2_marker(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let normalized = lowered.strip_prefix("bedrock/").unwrap_or(lowered.as_str());
    let Some(start) = normalized.find(TITAN_V2_EMBED_MODEL_MARKER) else {
        return false;
    };
    let end = start.saturating_add(TITAN_V2_EMBED_MODEL_MARKER.len());
    end == normalized.len() || matches!(normalized.as_bytes().get(end), Some(b':' | b'/'))
}
