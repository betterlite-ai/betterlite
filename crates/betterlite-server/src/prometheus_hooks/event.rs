use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

// the parts of LiteLLM's kwargs a hook reads, rebuilt from the typed event because LoggerEvent keeps no litellm_params
pub(crate) struct HookCall {
    pub(crate) model: String,
    pub(crate) litellm_params: JsonObject,
    pub(crate) metadata: IndexMap<String, Json>,
}

impl HookCall {
    pub(crate) fn from_event(event: &LoggerEvent) -> Self {
        let payload = &event.payload;
        let metadata = merged_metadata(event);
        let mut params = JsonObject::new();
        params.insert("metadata".to_owned(), Json::Object(metadata.clone()));
        params.insert(
            "litellm_metadata".to_owned(),
            Json::Object(metadata.clone()),
        );
        params.insert("model".to_owned(), Json::Text(payload.model.clone()));
        params.insert("api_base".to_owned(), Json::Text(payload.api_base.clone()));
        if let Some(provider) = &payload.custom_llm_provider {
            params.insert(
                "custom_llm_provider".to_owned(),
                Json::Text(provider.clone()),
            );
        }
        // the proxy stamps this on litellm_params itself, so it only survives when a producer copies it into the metadata
        if let Some(flag) = metadata.get("proxy_rejected_before_routing") {
            params.insert("proxy_rejected_before_routing".to_owned(), flag.clone());
        }
        Self {
            model: payload.model.clone(),
            litellm_params: params,
            metadata,
        }
    }
}

// the request metadata LiteLLM hands the hooks, with the typed payload fields filling the keys it lacks
fn merged_metadata(event: &LoggerEvent) -> IndexMap<String, Json> {
    let mut merged = (*event.litellm_metadata).clone();
    if let Ok(Json::Object(recorded)) =
        serde_json::to_value(&event.payload.metadata).and_then(serde_json::from_value::<Json>)
    {
        for (key, value) in recorded {
            if !matches!(value, Json::Null) {
                merged.entry(key).or_insert(value);
            }
        }
    }
    merge_model_info(&mut merged, event.payload.model_id.as_deref());
    if let Some(group) = event
        .payload
        .model_group
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        merged
            .entry("model_group".to_owned())
            .or_insert_with(|| Json::Text(group.to_owned()));
    }
    merged
}

// the router keeps the deployment id under model_info.id, and a caller's model_info keeps its other fields
fn merge_model_info(metadata: &mut IndexMap<String, Json>, model_id: Option<&str>) {
    let Some(model_id) = model_id.filter(|id| !id.is_empty()) else {
        return;
    };
    let entry = metadata
        .entry("model_info".to_owned())
        .or_insert_with(|| Json::Object(JsonObject::new()));
    if let Json::Object(fields) = entry {
        fields
            .entry("id".to_owned())
            .or_insert_with(|| Json::Text(model_id.to_owned()));
    }
}

// the budget fan-out reads the auth-carried snapshots out of a plain JSON map
pub(crate) fn fan_out_metadata(metadata: &IndexMap<String, Json>) -> BTreeMap<String, AgentJson> {
    let mut out = BTreeMap::new();
    if let Ok(encoded) = serde_json::to_value(Json::Object(metadata.clone()))
        && let Some(fields) = encoded.as_object()
    {
        for (key, value) in fields {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}
