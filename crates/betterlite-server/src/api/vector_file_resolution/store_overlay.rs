use betterlite_core::api::managed_resource_ids::{decode_unified_id, parse_unified_id};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::error::VectorFileResolutionError;
use super::{StoreDeps, has_content};

// _update_request_data_with_litellm_managed_vector_store_registry: a decoded store id routes and returns, otherwise the store record overlays its fields.
pub fn apply_store_overlay(
    data: &mut JsonObject,
    vector_store_id: &str,
    deps: &StoreDeps<'_>,
) -> Result<(), VectorFileResolutionError> {
    if decode_unified_id(vector_store_id).is_some() {
        apply_unified_store(data, vector_store_id);
        return Ok(());
    }
    let Some(store) = lookup_store(vector_store_id, deps) else {
        return Ok(());
    };
    let fields = store_fields(store)?;
    apply_store_fields(data, &fields)
}

// a truthy model_id before the first target name, and a truthy provider resource id replaces the request's store id.
fn apply_unified_store(data: &mut JsonObject, vector_store_id: &str) {
    let Some(parts) = parse_unified_id(vector_store_id) else {
        return;
    };
    let routing_model = match parts.model_id.filter(|model_id| !model_id.is_empty()) {
        Some(model_id) => Some(model_id),
        None => parts.target_model_names.first().cloned(),
    };
    if let Some(model) = routing_model.filter(|model| !model.is_empty()) {
        data.insert("model".to_owned(), Json::Text(model));
    }
    if let Some(provider_resource_id) = parts
        .provider_resource_id
        .filter(|resource_id| !resource_id.is_empty())
    {
        data.insert(
            "vector_store_id".to_owned(),
            Json::Text(provider_resource_id),
        );
    }
}

// The route's own store wins; the registry is read only when nothing was resolved and the route allows the lookup.
fn lookup_store<'store>(
    vector_store_id: &str,
    deps: &StoreDeps<'store>,
) -> Option<&'store LiteLlmManagedVectorStore> {
    if let Some(store) = deps.managed_store {
        return Some(store);
    }
    if !deps.should_lookup_registry {
        return None;
    }
    deps.registry?.store_by_id(vector_store_id)
}

// The record's JSON form: every key it carried, null and wrong-typed values included.
fn store_fields(
    store: &LiteLlmManagedVectorStore,
) -> Result<JsonObject, VectorFileResolutionError> {
    serde_json::to_value(store)
        .and_then(serde_json::from_value)
        .map_err(VectorFileResolutionError::Store)
}

// The two top-level copies happen even for null, then a truthy litellm_params overrides them and every request field.
fn apply_store_fields(
    data: &mut JsonObject,
    fields: &JsonObject,
) -> Result<(), VectorFileResolutionError> {
    for name in ["custom_llm_provider", "litellm_credential_name"] {
        if let Some(value) = fields.get(name) {
            data.insert(name.to_owned(), value.clone());
        }
    }
    let Some(params) = fields.get("litellm_params") else {
        return Ok(());
    };
    if !has_content(params) {
        return Ok(());
    }
    let Json::Object(entries) = params else {
        return Err(VectorFileResolutionError::StoreParams);
    };
    data.extend(entries.clone());
    Ok(())
}
