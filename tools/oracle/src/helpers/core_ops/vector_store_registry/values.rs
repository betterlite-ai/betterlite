use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde_json::{Value, json};

// the record projection both sides compare: the two clock stamps are left out because each side
// reads its own clock, and the native tests prove the injected instant
pub(super) fn stores_value(stores: &[LiteLlmManagedVectorStore]) -> Value {
    let mut values = Vec::with_capacity(stores.len());
    for store in stores {
        values.push(store_value(store));
    }
    Value::Array(values)
}

pub(super) fn store_value(store: &LiteLlmManagedVectorStore) -> Value {
    json!({
        "vector_store_id": text(store.vector_store_id.as_deref()),
        "custom_llm_provider": text(store.custom_llm_provider.as_deref()),
        "vector_store_name": text(store.vector_store_name.as_deref()),
        "vector_store_description": text(store.vector_store_description.as_deref()),
        "vector_store_metadata": payload(store.vector_store_metadata.as_ref()),
        "litellm_credential_name": text(store.litellm_credential_name.as_deref()),
        "litellm_params": payload(store.litellm_params.as_ref()),
        "team_id": text(store.team_id.as_deref()),
        "user_id": text(store.user_id.as_deref()),
        "is_config": flag(store.is_config),
    })
}

pub(super) fn indexes_value(indexes: &[LiteLlmManagedVectorStoreIndex]) -> Value {
    let mut values = Vec::with_capacity(indexes.len());
    for index in indexes {
        values.push(index_value(index));
    }
    Value::Array(values)
}

pub(super) fn index_value(index: &LiteLlmManagedVectorStoreIndex) -> Value {
    let info = match &index.index_info {
        Some(info) => json!(info),
        None => Value::Null,
    };
    json!({
        "id": &index.id,
        "index_name": &index.index_name,
        "litellm_params": {
            "vector_store_index": &index.litellm_params.vector_store_index,
            "vector_store_name": &index.litellm_params.vector_store_name,
        },
        "index_info": info,
        "created_by": text(index.created_by.as_deref()),
        "updated_by": text(index.updated_by.as_deref()),
    })
}

pub(super) fn object_value(value: Option<&JsonObject>) -> Value {
    match value {
        Some(fields) => json!(fields),
        None => Value::Null,
    }
}

fn text(value: Option<&str>) -> Value {
    match value {
        Some(found) => Value::String(found.to_owned()),
        None => Value::Null,
    }
}

fn flag(value: Option<bool>) -> Value {
    match value {
        Some(enabled) => Value::Bool(enabled),
        None => Value::Null,
    }
}

fn payload(value: Option<&Value>) -> Value {
    match value {
        Some(found) => found.clone(),
        None => Value::Null,
    }
}
