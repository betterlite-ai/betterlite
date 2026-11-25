use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::error::VectorManagementError;

// _with_ownership: a record without is_config reads as false, and a present true, false or null stays as it came
#[must_use]
pub fn with_ownership(record: &LiteLlmManagedVectorStore) -> LiteLlmManagedVectorStore {
    let mut owned = record.clone();
    if owned.is_config.is_none() && !owned.extra.contains_key("is_config") {
        owned.is_config = Some(false);
    }
    owned
}

// _raise_if_config_owned: only the registry says a config file owns the id, and no registry owns nothing
pub fn raise_if_config_owned(
    vector_store_id: &str,
    registry: Option<&VectorStoreRegistry>,
) -> Result<(), VectorManagementError> {
    let Some(registry) = registry else {
        return Ok(());
    };
    if registry.is_config(vector_store_id) {
        return Err(VectorManagementError::ConfigOwned {
            vector_store_id: vector_store_id.to_owned(),
        });
    }
    Ok(())
}
