use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde_json::{Value, json};

// the record projection both sides compare: the two clock stamps are left out because each side reads its own clock
pub(super) fn stores_value(stores: &[LiteLlmManagedVectorStore]) -> Value {
    let mut values = Vec::with_capacity(stores.len());
    for store in stores {
        values.push(store_value(store));
    }
    Value::Array(values)
}

pub(super) fn store_value(store: &LiteLlmManagedVectorStore) -> Value {
    json!({
        "vector_store_id": &store.vector_store_id,
        "custom_llm_provider": &store.custom_llm_provider,
        "vector_store_name": &store.vector_store_name,
        "vector_store_description": &store.vector_store_description,
        "vector_store_metadata": &store.vector_store_metadata,
        "litellm_credential_name": &store.litellm_credential_name,
        "litellm_params": &store.litellm_params,
        "team_id": &store.team_id,
        "user_id": &store.user_id,
        "is_config": &store.is_config,
    })
}

// the index projection: the index_info map and both audit names travel as stored
pub(super) fn indexes_value(indexes: &[LiteLlmManagedVectorStoreIndex]) -> Value {
    let mut values = Vec::with_capacity(indexes.len());
    for index in indexes {
        values.push(index_value(index));
    }
    Value::Array(values)
}

pub(super) fn index_value(index: &LiteLlmManagedVectorStoreIndex) -> Value {
    let params = json!({
        "vector_store_index": &index.litellm_params.vector_store_index,
        "vector_store_name": &index.litellm_params.vector_store_name,
    });
    json!({
        "id": &index.id,
        "index_name": &index.index_name,
        "litellm_params": params,
        "index_info": &index.index_info,
        "created_by": &index.created_by,
        "updated_by": &index.updated_by,
    })
}
