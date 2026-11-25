use std::collections::BTreeMap;
use std::io;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_resource_grants::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreCache, ManagedVectorStoreRows, VECTOR_STORE_CACHE_PREFIX,
    VectorStoreRow, VectorStoreRowsFailure, project_row,
};
use betterlite_core::api::managed_store_access::lookup::{
    ManagedStoreLookup, ManagedStoreRegistry,
};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use serde_json::{Value, json};

use super::{Failure, RegistryInput};

// what a scripted failure raises, so both sides warn about the same arm
const REGISTRY_FAILURE: &str = "the case scripts a registry failure";
const CACHE_FAILURE: &str = "the case scripts a cache failure";
const CACHE_TTL_SECONDS: f64 = 60.0;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct RegistryBroke(pub(super) String);

pub(super) struct ScriptedRegistry {
    stores: Vec<LiteLlmManagedVectorStore>,
    failing: bool,
    looks: Mutex<Vec<String>>,
}

impl ManagedStoreRegistry for ScriptedRegistry {
    type Error = RegistryBroke;

    fn lookup(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, RegistryBroke> {
        lock(&self.looks).push(vector_store_id.to_owned());
        if self.failing {
            return Err(RegistryBroke(String::from(REGISTRY_FAILURE)));
        }
        let found = self
            .stores
            .iter()
            .find(|store| store.vector_store_id.as_deref() == Some(vector_store_id));
        Ok(found.cloned())
    }
}

pub(super) struct ScriptedCache {
    hits: BTreeMap<String, ManagedVectorStoresTable>,
    failure: Option<Failure>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<String>>,
}

#[async_trait]
impl ManagedVectorStoreCache for ScriptedCache {
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError> {
        lock(&self.reads).push(key.to_owned());
        if self.failure == Some(Failure::CacheRead) {
            return Err(PayloadError::Json(serde_json::Error::io(io::Error::other(
                CACHE_FAILURE,
            ))));
        }
        Ok(self.hits.get(key).cloned())
    }

    fn management_ttl(&self) -> f64 {
        CACHE_TTL_SECONDS
    }

    async fn write(
        &self,
        key: &str,
        _row: &ManagedVectorStoresTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        if self.failure == Some(Failure::CacheWrite) {
            return Err(AuthCacheError::from(PayloadError::Json(
                serde_json::Error::io(io::Error::other(CACHE_FAILURE)),
            )));
        }
        lock(&self.writes).push(key.to_owned());
        Ok(())
    }
}

pub(super) struct ScriptedRows {
    rows: Vec<VectorStoreRow>,
    queries: Mutex<Vec<(Vec<String>, i64)>>,
}

#[async_trait]
impl ManagedVectorStoreRows for ScriptedRows {
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure> {
        lock(&self.queries).push((query.uuids, query.take));
        Ok(self.rows.clone())
    }
}

// the scripted registry, cache and row source behind one resolution
pub(super) struct World {
    registry: Option<ScriptedRegistry>,
    cache: ScriptedCache,
    rows: ScriptedRows,
    database: bool,
}

impl World {
    pub(super) fn build(
        registry: Option<&RegistryInput>,
        database: bool,
        cached: &[VectorStoreRow],
        rows: &[VectorStoreRow],
        failure: Option<Failure>,
    ) -> Result<Self, Box<HelperError>> {
        let mut hits = BTreeMap::new();
        for row in cached {
            let Some(table) = project_row(row).or_value_error()? else {
                continue;
            };
            let key = format!("{VECTOR_STORE_CACHE_PREFIX}{}", table.vector_store_id);
            hits.insert(key, table);
        }
        let registry = registry.map(|scripted| ScriptedRegistry {
            stores: scripted.stores.clone(),
            failing: failure == Some(Failure::Registry),
            looks: Mutex::new(Vec::new()),
        });
        Ok(Self {
            registry,
            cache: ScriptedCache {
                hits,
                failure,
                reads: Mutex::new(Vec::new()),
                writes: Mutex::new(Vec::new()),
            },
            rows: ScriptedRows {
                rows: rows.to_vec(),
                queries: Mutex::new(Vec::new()),
            },
            database,
        })
    }

    pub(super) fn lookup(&self) -> ManagedStoreLookup<'_, ScriptedRegistry> {
        ManagedStoreLookup {
            registry: self.registry.as_ref(),
            database: self.database,
            cache: &self.cache,
            rows: &self.rows,
        }
    }

    // every library lookup the call reached, in the order it ran
    pub(super) fn lookups(&self) -> Value {
        let registry = match &self.registry {
            Some(scripted) => lock(&scripted.looks).clone(),
            None => Vec::new(),
        };
        let queries = lock(&self.rows.queries).clone();
        json!({
            "registry": registry,
            "cache_reads": lock(&self.cache.reads).clone(),
            "cache_writes": lock(&self.cache.writes).clone(),
            "row_queries": queries
                .into_iter()
                .map(|(uuids, take)| json!({"uuids": uuids, "take": take}))
                .collect::<Vec<Value>>(),
        })
    }
}
