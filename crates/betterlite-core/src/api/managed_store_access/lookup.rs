use std::convert::Infallible;
use std::error::Error;

use betterlite_auth::auth_resource_grants::managed_rows::{
    ManagedVectorStoreCache, ManagedVectorStoreRows, vector_store_rows_by_uuids,
};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use crate::api::vector_store_access::store::{ManagedVectorStore, normalize_litellm_params};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

use super::error::{ManagedStoreAccessError, ResolutionSource};

// get_litellm_managed_vector_store_from_registry, asked of whichever registry the server holds
pub trait ManagedStoreRegistry: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn lookup(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, Self::Error>;
}

// B1062's registry answers from its loaded stores and has no failure to report
impl ManagedStoreRegistry for VectorStoreRegistry {
    type Error = Infallible;

    fn lookup(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, Infallible> {
        Ok(self.store_by_id(vector_store_id).cloned())
    }
}

// the registry, the shared cache and the row source one resolution reads
pub struct ManagedStoreLookup<'lookup, Registry> {
    pub registry: Option<&'lookup Registry>,
    pub database: bool,
    pub cache: &'lookup dyn ManagedVectorStoreCache,
    pub rows: &'lookup dyn ManagedVectorStoreRows,
}

// LiteLLM warns with the same text it fails closed on, from the registry or the shared cache
fn resolution_failure<RegistryError>(
    vector_store_id: &str,
    source: ResolutionSource<RegistryError>,
    from_registry: bool,
) -> ManagedStoreAccessError<RegistryError>
where
    RegistryError: Error + 'static,
{
    let owned_id = String::from(vector_store_id);
    let error = if from_registry {
        ManagedStoreAccessError::RegistryFailure {
            vector_store_id: owned_id,
            source,
        }
    } else {
        ManagedStoreAccessError::SharedCacheFailure {
            vector_store_id: owned_id,
            source,
        }
    };
    tracing::warn!("{error}");
    error
}

// get_litellm_managed_vector_store: the registry first, then the shared cache and the database
pub async fn get_managed_vector_store<Registry>(
    lookup: &ManagedStoreLookup<'_, Registry>,
    vector_store_id: &str,
) -> Result<Option<ManagedVectorStore>, ManagedStoreAccessError<Registry::Error>>
where
    Registry: ManagedStoreRegistry,
{
    if vector_store_id.is_empty() {
        return Ok(None);
    }
    if let Some(registry) = lookup.registry {
        match registry.lookup(vector_store_id) {
            Ok(Some(store)) => {
                return normalize_litellm_params(&ManagedVectorStore(store))
                    .map(Some)
                    .map_err(|source| {
                        resolution_failure(vector_store_id, ResolutionSource::Params(source), true)
                    });
            }
            Ok(None) => {}
            Err(source) => {
                return Err(resolution_failure(
                    vector_store_id,
                    ResolutionSource::Registry(source),
                    true,
                ));
            }
        }
    }
    resolve_from_shared_cache(lookup, vector_store_id).await
}

// the shared cache and database arm takes the first row and normalizes it like a registry hit
async fn resolve_from_shared_cache<Registry>(
    lookup: &ManagedStoreLookup<'_, Registry>,
    vector_store_id: &str,
) -> Result<Option<ManagedVectorStore>, ManagedStoreAccessError<Registry::Error>>
where
    Registry: ManagedStoreRegistry,
{
    let uuids = [String::from(vector_store_id)];
    let rows = vector_store_rows_by_uuids(&uuids, lookup.database, lookup.cache, lookup.rows)
        .await
        .map_err(|source| {
            resolution_failure(vector_store_id, ResolutionSource::Lookup(source), false)
        })?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    // the rows are typed, so this arm guards the record's deserializer, not stored input
    let record: LiteLlmManagedVectorStore = serde_json::to_value(row)
        .and_then(serde_json::from_value)
        .map_err(|source| {
            resolution_failure(vector_store_id, ResolutionSource::Record(source), false)
        })?;
    normalize_litellm_params(&ManagedVectorStore(record))
        .map(Some)
        .map_err(|source| {
            resolution_failure(vector_store_id, ResolutionSource::Params(source), false)
        })
}
