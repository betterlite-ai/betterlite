use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_server::api::vector_management_write::cache::{
    CacheError, RegistryCache, VectorStoreCache,
};

// the registry the writes mutate, with every call the endpoint makes recorded in order
pub(super) struct LoggingCache {
    pub(super) inner: RegistryCache,
    pub(super) events: Arc<Mutex<Vec<String>>>,
}

impl LoggingCache {
    fn note(&self, entry: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

impl VectorStoreCache for LoggingCache {
    fn find(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        self.note(format!("cache:find:{vector_store_id}"));
        self.inner.find(vector_store_id)
    }

    fn add(&self, store: LiteLlmManagedVectorStore) -> Result<(), CacheError> {
        self.note(format!(
            "cache:add:{}",
            store.vector_store_id.clone().unwrap_or_default()
        ));
        self.inner.add(store)
    }

    fn update(
        &self,
        vector_store_id: &str,
        store: LiteLlmManagedVectorStore,
    ) -> Result<(), CacheError> {
        self.note(format!("cache:update:{vector_store_id}"));
        self.inner.update(vector_store_id, store)
    }

    fn remove(&self, vector_store_id: &str) -> Result<(), CacheError> {
        self.note(format!("cache:remove:{vector_store_id}"));
        self.inner.remove(vector_store_id)
    }
}
