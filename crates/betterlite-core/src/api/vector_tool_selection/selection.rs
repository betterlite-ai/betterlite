use indexmap::IndexMap;

use betterlite_models::call_helpers::tool_calls::remove_items_at_indices;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use crate::api::vector_store_registry::error::VectorStoreRegistryError;
use crate::api::vector_store_registry::repository::{VectorStoreDatabase, rows_from_database};
use crate::api::vector_store_registry::store::VectorStoreRegistry;
use crate::api::vector_store_registry::tools::{
    VectorStoreToolParams, extract_tool_params, ids_to_run,
};

use super::database::VectorStoreSelectionDatabase;
use super::error::VectorToolSelectionError;

// get_and_pop_recognised_vector_store_tools: a tool is recognized when every id it carries is in memory, and removal is deferred so a failed extraction leaves the list whole
pub fn pop_recognised_tools(
    registry: &VectorStoreRegistry,
    tools: &mut Vec<JsonObject>,
    vector_store_ids: &mut Vec<String>,
) -> Result<IndexMap<String, VectorStoreToolParams>, VectorToolSelectionError> {
    let mut params_by_id = IndexMap::new();
    if tools.is_empty() {
        return Ok(params_by_id);
    }
    let mut positions = Vec::new();
    for (position, tool) in tools.iter().enumerate() {
        let Some(value) = tool.get("vector_store_ids") else {
            continue;
        };
        let tool_ids = string_ids(value, "tool.vector_store_ids")?;
        if tool_ids.is_empty() {
            continue;
        }
        if !tool_ids.iter().all(|id| registry.store_by_id(id).is_some()) {
            continue;
        }
        vector_store_ids.extend(tool_ids.iter().cloned());
        let params = extract_tool_params(tool)?;
        for id in &tool_ids {
            params_by_id.insert(id.clone(), params.clone());
        }
        let Ok(index) = i64::try_from(position) else {
            continue;
        };
        positions.push(index);
    }
    remove_items_at_indices(tools, &positions);
    Ok(params_by_id)
}

// get_vector_store_to_run: B1062's stable deduplicated ids decide which stored record answers first
pub fn get_vector_store_to_run(
    registry: &VectorStoreRegistry,
    non_default_params: &JsonObject,
    tools: Option<&[JsonObject]>,
) -> Result<Option<LiteLlmManagedVectorStore>, VectorToolSelectionError> {
    let ids = ids_to_run(non_default_params, tools)?;
    let found = ids.iter().find_map(|id| registry.store_by_id(id));
    Ok(found.cloned())
}

// get_litellm_managed_vector_store_from_registry_by_name: the first stored record carrying the name answers, with no database fallback
#[must_use]
pub fn get_vector_store_by_name(
    registry: &VectorStoreRegistry,
    vector_store_name: &str,
) -> Option<LiteLlmManagedVectorStore> {
    let found = registry
        .stores
        .iter()
        .find(|store| store.vector_store_name.as_deref() == Some(vector_store_name));
    found.cloned()
}

// pop_vector_stores_to_run_with_db_fallback: request ids keep their duplicates and order, database-owned cache entries are verified, and stored litellm_params wins over tool params
pub async fn pop_stores_with_fallback<D>(
    registry: &mut VectorStoreRegistry,
    non_default_params: &mut JsonObject,
    tools: Option<&mut Vec<JsonObject>>,
    database: Option<&D>,
) -> Result<Vec<LiteLlmManagedVectorStore>, VectorToolSelectionError>
where
    D: VectorStoreSelectionDatabase,
{
    let mut ids = match non_default_params.shift_remove("vector_store_ids") {
        None => Vec::new(),
        Some(value) => string_ids(&value, "vector_store_ids")?,
    };
    let params_by_id = match tools {
        None => IndexMap::new(),
        Some(tools) => pop_recognised_tools(registry, tools, &mut ids)?,
    };
    let mut selected = Vec::new();
    for id in &ids {
        let mut store = match database {
            None => registry.store_by_id(id).cloned(),
            Some(database) => select_verified_store(registry, id, database).await,
        };
        if let (Some(record), Some(params)) = (&mut store, params_by_id.get(id)) {
            merge_tool_params(record, params)?;
        }
        if let Some(record) = store {
            selected.push(record);
        }
    }
    Ok(selected)
}

async fn select_verified_store<D>(
    registry: &mut VectorStoreRegistry,
    vector_store_id: &str,
    database: &D,
) -> Option<LiteLlmManagedVectorStore>
where
    D: VectorStoreSelectionDatabase,
{
    let Some(cached) = registry.store_by_id(vector_store_id).cloned() else {
        return fallback_store(registry, vector_store_id, database).await;
    };
    if registry.is_config(vector_store_id) {
        return Some(cached);
    }
    match database.find_store(vector_store_id).await {
        Ok(Some(_)) => Some(cached),
        Ok(None) => {
            tracing::debug!(
                "Vector store {vector_store_id} found in memory but deleted from database, removing from cache"
            );
            registry.remove_store(vector_store_id);
            fallback_store(registry, vector_store_id, database).await
        }
        Err(error) => {
            tracing::debug!("Error verifying vector store {vector_store_id} in database: {error}");
            Some(cached)
        }
    }
}

// B1062's get_litellm_managed_vector_store_from_registry_or_db reads every row and caches the first match
async fn fallback_store<D>(
    registry: &mut VectorStoreRegistry,
    vector_store_id: &str,
    database: &D,
) -> Option<LiteLlmManagedVectorStore>
where
    D: VectorStoreDatabase,
{
    let rows = match rows_from_database(Some(database)).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::debug!("Error fetching vector store {vector_store_id} from database: {error}");
            return None;
        }
    };
    let found = rows
        .into_iter()
        .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id));
    if let Some(row) = &found {
        registry.add_store(row.clone());
    }
    found
}

// VectorStoreToolParams.to_dict drops absent options and the stored value overwrites each shared key
fn merge_tool_params(
    record: &mut LiteLlmManagedVectorStore,
    params: &VectorStoreToolParams,
) -> Result<(), VectorToolSelectionError> {
    let mut tool_fields = JsonObject::new();
    if let Some(filters) = &params.filters {
        tool_fields.insert("filters".to_owned(), Json::Object(filters.clone()));
    }
    if let Some(max_num_results) = params.max_num_results {
        tool_fields.insert(
            "max_num_results".to_owned(),
            Json::Number(serde_json::Number::from(max_num_results)),
        );
    }
    if let Some(ranking_options) = &params.ranking_options {
        tool_fields.insert(
            "ranking_options".to_owned(),
            Json::Object(ranking_options.clone()),
        );
    }
    let tool_params = serde_json::to_value(Json::Object(tool_fields))?;
    let Some(mut stored) = record.litellm_params.take() else {
        record.litellm_params = Some(tool_params);
        return Ok(());
    };
    let Some(stored_fields) = stored.as_object_mut() else {
        return Err(VectorToolSelectionError::StoredParams);
    };
    if let Some(tool_values) = tool_params.as_object() {
        for (name, value) in tool_values {
            stored_fields
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
    }
    record.litellm_params = Some(stored);
    Ok(())
}

fn string_ids(value: &Json, field: &'static str) -> Result<Vec<String>, VectorStoreRegistryError> {
    match value {
        Json::Null => Ok(Vec::new()),
        Json::List(items) => {
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Json::Text(id) => ids.push(id.clone()),
                    Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_)
                    | Json::Null => {
                        return Err(VectorStoreRegistryError::IdList { field });
                    }
                }
            }
            Ok(ids)
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(VectorStoreRegistryError::IdList { field })
        }
    }
}
