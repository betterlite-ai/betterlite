use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CacheError {
    pub message: String,
}

// the in-memory registry as the writes reach it: every mutation can fail, so a broken registry surfaces as the route's 500
pub trait VectorStoreCache: Send + Sync {
    fn find(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore>;

    fn add(&self, store: LiteLlmManagedVectorStore) -> Result<(), CacheError>;

    fn update(
        &self,
        vector_store_id: &str,
        store: LiteLlmManagedVectorStore,
    ) -> Result<(), CacheError>;

    fn remove(&self, vector_store_id: &str) -> Result<(), CacheError>;
}

// B1062's registry behind the write port: the same object the reads and the rest of the proxy share
pub struct RegistryCache(pub Arc<Mutex<VectorStoreRegistry>>);

impl VectorStoreCache for RegistryCache {
    fn find(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store_by_id(vector_store_id)
            .cloned()
    }

    fn add(&self, store: LiteLlmManagedVectorStore) -> Result<(), CacheError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .add_store(store);
        Ok(())
    }

    fn update(
        &self,
        vector_store_id: &str,
        store: LiteLlmManagedVectorStore,
    ) -> Result<(), CacheError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .update_store(vector_store_id, store);
        Ok(())
    }

    fn remove(&self, vector_store_id: &str) -> Result<(), CacheError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove_store(vector_store_id);
        Ok(())
    }
}
