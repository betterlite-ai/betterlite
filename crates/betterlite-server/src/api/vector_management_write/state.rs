use std::sync::{Arc, PoisonError};

use super::cache::{RegistryCache, VectorStoreCache};
use super::error::ManagementWriteError;
use crate::api::vector_management_boundaries::ownership::raise_if_config_owned;
use crate::api::vector_management_read::state::VectorManagementReadState;

// the reads' stage plus the one in-memory registry the writes mutate
pub struct VectorManagementWriteState<Database> {
    pub reads: Arc<VectorManagementReadState<Database>>,
    // None is LiteLLM's litellm.vector_store_registry is None: no config ownership and no cache writes
    pub cache: Option<Arc<dyn VectorStoreCache>>,
}

impl<Database> VectorManagementWriteState<Database> {
    // _raise_if_config_owned: only the registry says a config file owns the id, and no registry owns nothing
    pub fn refuse_config_owned(&self, vector_store_id: &str) -> Result<(), ManagementWriteError> {
        let Some(registry) = &self.reads.registry else {
            return Ok(());
        };
        let stores = registry.lock().unwrap_or_else(PoisonError::into_inner);
        raise_if_config_owned(vector_store_id, Some(&stores))?;
        Ok(())
    }
}

// the registry the reads already carry becomes the write cache, so both routes see one registry
impl<Database> From<Arc<VectorManagementReadState<Database>>>
    for VectorManagementWriteState<Database>
{
    fn from(reads: Arc<VectorManagementReadState<Database>>) -> Self {
        let registry = reads.registry.as_ref();
        let cache = registry.map(|registry| {
            let cache: Arc<dyn VectorStoreCache> = Arc::new(RegistryCache(Arc::clone(registry)));
            cache
        });
        Self { reads, cache }
    }
}
