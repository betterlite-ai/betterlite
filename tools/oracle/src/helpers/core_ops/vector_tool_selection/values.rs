use betterlite_core::api::vector_store_registry::tools::VectorStoreToolParams;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};

// the ten stored fields both sides compare, absent ones as null; the two clock stamps stay out because each side reads its own clock
#[derive(Serialize)]
struct StoreFields<'store> {
    vector_store_id: Option<&'store str>,
    custom_llm_provider: Option<&'store str>,
    vector_store_name: Option<&'store str>,
    vector_store_description: Option<&'store str>,
    vector_store_metadata: Option<&'store Value>,
    litellm_credential_name: Option<&'store str>,
    litellm_params: Option<&'store Value>,
    team_id: Option<&'store str>,
    user_id: Option<&'store str>,
    is_config: Option<bool>,
}

pub(super) fn stores_value(stores: &[LiteLlmManagedVectorStore]) -> Value {
    let mut values = Vec::with_capacity(stores.len());
    for store in stores {
        values.push(store_value(store));
    }
    Value::Array(values)
}

pub(super) fn store_value(store: &LiteLlmManagedVectorStore) -> Value {
    let fields = StoreFields {
        vector_store_id: store.vector_store_id.as_deref(),
        custom_llm_provider: store.custom_llm_provider.as_deref(),
        vector_store_name: store.vector_store_name.as_deref(),
        vector_store_description: store.vector_store_description.as_deref(),
        vector_store_metadata: store.vector_store_metadata.as_ref(),
        litellm_credential_name: store.litellm_credential_name.as_deref(),
        litellm_params: store.litellm_params.as_ref(),
        team_id: store.team_id.as_deref(),
        user_id: store.user_id.as_deref(),
        is_config: store.is_config,
    };
    json!(fields)
}

pub(super) fn params_map(records: &IndexMap<String, VectorStoreToolParams>) -> Value {
    let mut fields = serde_json::Map::new();
    for (id, params) in records {
        fields.insert(id.clone(), params_value(params));
    }
    Value::Object(fields)
}

// VectorStoreToolParams.to_dict: only the options the tool declared travel, absent ones stay out
fn params_value(params: &VectorStoreToolParams) -> Value {
    let mut fields = serde_json::Map::new();
    if let Some(filters) = &params.filters {
        fields.insert("filters".to_owned(), json!(filters));
    }
    if let Some(max_num_results) = params.max_num_results {
        fields.insert("max_num_results".to_owned(), json!(max_num_results));
    }
    if let Some(ranking_options) = &params.ranking_options {
        fields.insert("ranking_options".to_owned(), json!(ranking_options));
    }
    Value::Object(fields)
}
