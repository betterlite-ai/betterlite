use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_access::store::ManagedVectorStore;
use betterlite_core::api::vector_store_registry::repository::{
    VectorStoreDatabase, rows_from_database,
};
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    LiteLlmManagedVectorStore, ManagedVectorStoreList,
};
use indexmap::IndexMap;

use super::caller::CallerView;
use super::error::ManagementReadError;
use super::state::VectorManagementReadState;
use crate::api::vector_management_boundaries::ownership::with_ownership;
use crate::api::vector_management_boundaries::redaction::redact_litellm_params;

// the fields LiteLLM_ManagedVectorStore declares, so the response model keeps these keys and drops any other
const DECLARED_KEYS: [&str; 12] = [
    "vector_store_id",
    "custom_llm_provider",
    "vector_store_name",
    "vector_store_description",
    "vector_store_metadata",
    "created_at",
    "updated_at",
    "litellm_credential_name",
    "litellm_params",
    "team_id",
    "user_id",
    "is_config",
];

// list_vector_stores: the feature switch outside the try, then every failure inside becomes the catch-all 500
pub async fn list_vector_stores<Database>(
    state: &VectorManagementReadState<Database>,
    caller: &CallerView,
    page: i64,
    page_size: i64,
) -> Result<ManagedVectorStoreList, ManagementReadError>
where
    Database: VectorStoreDatabase + Send + Sync,
{
    state.check_vector_feature(caller).await?;
    reconciled_list(state, caller, page, page_size)
        .await
        .map_err(|error| {
            tracing::error!("Error listing vector stores: {error}");
            ManagementReadError::Internal {
                message: error.to_string(),
            }
        })
}

async fn reconciled_list<Database>(
    state: &VectorManagementReadState<Database>,
    caller: &CallerView,
    page: i64,
    page_size: i64,
) -> Result<ManagedVectorStoreList, ManagementReadError>
where
    Database: VectorStoreDatabase + Send + Sync,
{
    let stores = reconciled_stores(state).await?;
    let candidates: Vec<ManagedVectorStore> =
        stores.into_values().map(ManagedVectorStore).collect();
    let listable = state
        .access()
        .filter_listable_vector_stores(&candidates, &caller.key)
        .await
        .map_err(|error| ManagementReadError::Internal {
            message: error.to_string(),
        })?;
    let mut data = Vec::with_capacity(listable.len());
    for store in &listable {
        data.push(list_item(&store.0, state.limits)?);
    }
    let total_count = i64::try_from(data.len()).unwrap_or(i64::MAX);
    let pages = total_pages(data.len(), page_size)?;
    Ok(ManagedVectorStoreList {
        object: Some("list".to_owned()),
        data: Some(data),
        total_count: Some(total_count),
        current_page: Some(page),
        total_pages: Some(pages),
    })
}

// the DB rows in database order, then the registry's config entries and its not-yet-known records
async fn reconciled_stores<Database>(
    state: &VectorManagementReadState<Database>,
) -> Result<IndexMap<String, LiteLlmManagedVectorStore>, ManagementReadError>
where
    Database: VectorStoreDatabase + Send + Sync,
{
    let database_rows: Vec<LiteLlmManagedVectorStore> = rows_from_database(state.database.as_ref())
        .await
        .map_err(|error| ManagementReadError::Internal {
            message: error.to_string(),
        })?;
    let mut stores: IndexMap<String, LiteLlmManagedVectorStore> = IndexMap::new();
    let mut database_ids: BTreeSet<String> = BTreeSet::new();
    for row in &database_rows {
        if let Some(id) = present_id(row) {
            database_ids.insert(id.to_owned());
            stores.insert(id.to_owned(), row.clone());
        }
    }
    if let Some(registry) = &state.registry {
        reconcile(registry, &database_rows, &database_ids, &mut stores);
    }
    Ok(stores)
}

// the registry half: config entries win, deleted ids leave memory, and every DB row updates memory in order
fn reconcile(
    registry: &Mutex<VectorStoreRegistry>,
    database_rows: &[LiteLlmManagedVectorStore],
    database_ids: &BTreeSet<String>,
    stores: &mut IndexMap<String, LiteLlmManagedVectorStore>,
) {
    let mut locked = registry.lock().unwrap_or_else(PoisonError::into_inner);
    let in_memory = locked.stores.clone();
    let mut to_delete: Vec<String> = Vec::new();
    for store in &in_memory {
        let Some(id) = present_id(store) else {
            continue;
        };
        if store.is_config == Some(true) {
            stores.insert(id.to_owned(), store.clone());
        } else if !database_ids.contains(id) {
            tracing::info!(
                "Vector store {id} exists in memory but not in database - marking for deletion from cache"
            );
            to_delete.push(id.to_owned());
        } else if !stores.contains_key(id) {
            stores.insert(id.to_owned(), store.clone());
        }
    }
    for id in &to_delete {
        locked.remove_store(id);
        tracing::debug!("Removed deleted vector store {id} from in-memory registry");
    }
    for row in database_rows {
        if let Some(id) = present_id(row) {
            locked.update_store(id, row.clone());
        }
    }
}

fn present_id(record: &LiteLlmManagedVectorStore) -> Option<&str> {
    let id = record.vector_store_id.as_deref()?;
    (!id.is_empty()).then_some(id)
}

// _with_ownership plus the redacted params the list always carries, reduced to the fields the response declares
fn list_item(
    record: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<LiteLlmManagedVectorStore, ManagementReadError> {
    let mut item = with_ownership(record);
    let params = match &item.litellm_params {
        Some(value) => {
            let stored: Json = serde_json::from_value(value.clone())?;
            redact_litellm_params(&stored, 0, limits)
        }
        None => Json::Null,
    };
    item.litellm_params = Some(serde_json::to_value(params)?);
    item.extra
        .retain(|key, _value| key != "litellm_params" && DECLARED_KEYS.contains(&key.as_str()));
    Ok(item)
}

// (total_count + page_size - 1) // page_size, Python's floor division, and ZeroDivisionError's exact text
fn total_pages(total_count: usize, page_size: i64) -> Result<i64, ManagementReadError> {
    if page_size == 0 {
        return Err(ManagementReadError::Internal {
            message: "integer division or modulo by zero".to_owned(),
        });
    }
    let size = i128::from(page_size);
    let numerator = i128::try_from(total_count)
        .unwrap_or(i128::MAX)
        .saturating_add(size)
        .saturating_sub(1);
    let quotient = numerator.checked_div(size).unwrap_or(0);
    let remainder = numerator.checked_rem(size).unwrap_or(0);
    let floor_adjust = remainder != 0 && (remainder.is_negative() != size.is_negative());
    let floored = if floor_adjust {
        quotient.saturating_sub(1)
    } else {
        quotient
    };
    Ok(i64::try_from(floored).unwrap_or(i64::MAX))
}
