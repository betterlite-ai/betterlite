use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::identity::anthropic_text::anthropic_text_provider;
use betterlite_models::identity::cohere::cohere_chat_provider;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

// the parts of LiteLLM's kwargs a hook reads, rebuilt from the typed event because LoggerEvent keeps no litellm_params
pub(crate) struct HookCall {
    pub(crate) model: String,
    pub(crate) litellm_params: JsonObject,
    pub(crate) metadata: IndexMap<String, Json>,
}

impl HookCall {
    pub(crate) fn from_event(event: &LoggerEvent, catalog: &ModelCatalog) -> Self {
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
            model: labelled_model(payload, catalog),
            litellm_params: params,
            metadata,
        }
    }
}

// LiteLLM labels kwargs["model"], the name get_llm_provider returned, not the payload's rebuilt name
fn labelled_model(payload: &StandardLoggingPayload, catalog: &ModelCatalog) -> String {
    // the handlers rename a declared provider and return the model without its prefix
    let custom = payload.custom_llm_provider.as_deref();
    let (cohere_model, cohere_provider) = cohere_chat_provider(&payload.model, custom, catalog);
    let (model, provider) = anthropic_text_provider(&cohere_model, cohere_provider.as_deref());
    let resolved = provider.unwrap_or_default();
    // the payload records the resolved provider, so the declared prefix is the rename it replaced
    let declared = match resolved.as_str() {
        "cohere_chat" => "cohere",
        "anthropic_text" => "anthropic",
        "ai21_chat" => "ai21",
        "" => return model,
        other => other,
    };
    let rest = [declared, resolved.as_str()]
        .into_iter()
        .find_map(|prefix| model.strip_prefix(prefix)?.strip_prefix('/'));
    // get_llm_provider keeps a native OpenRouter id such as "openrouter/auto" whole
    let label = rest.filter(|tail| resolved != "openrouter" || tail.contains('/'));
    label.unwrap_or(&model).to_owned()
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
    let model_group = event.payload.model_group.as_deref();
    if let Some(group) = model_group.filter(|name| !name.is_empty()) {
        merged
            .entry("model_group".to_owned())
            .or_insert_with(|| Json::Text(group.to_owned()));
    }
    merged
}

// the router keeps the deployment id under model_info.id, and a caller's model_info keeps its other fields
fn merge_model_info(metadata: &mut IndexMap<String, Json>, model_id: Option<&str>) {
    let Some(deployment_id) = model_id.filter(|id| !id.is_empty()) else {
        return;
    };
    let entry = metadata
        .entry("model_info".to_owned())
        .or_insert_with(|| Json::Object(JsonObject::new()));
    if let Json::Object(fields) = entry {
        fields
            .entry("id".to_owned())
            .or_insert_with(|| Json::Text(deployment_id.to_owned()));
    }
}

// the budget fan-out reads the auth-carried snapshots out of a plain JSON map
pub(crate) fn fan_out_metadata(metadata: &IndexMap<String, Json>) -> BTreeMap<String, AgentJson> {
    let Ok(encoded) = serde_json::to_value(Json::Object(metadata.clone())) else {
        return BTreeMap::new();
    };
    let Some(fields) = encoded.as_object() else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
