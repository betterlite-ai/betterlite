use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use serde::Deserialize;

use super::rows::RowError;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};
use crate::auth_read::table::AuthReadError;

// LiteLLM reads and writes one row per UUID under this prefix with the management-object TTL
pub const VECTOR_STORE_CACHE_PREFIX: &str = "managed_vector_store_id:";

// the row a Prisma fixture hands back before LiteLLM projects it: model_dump, else dict, then the iterable form
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VectorStoreRow {
    pub model_dump: Option<JsonData>,
    pub dict: Option<JsonData>,
    pub fields: BTreeMap<String, JsonData>,
}

pub struct ManagedRowsQuery {
    pub uuids: Vec<String>,
    pub take: i64,
}

#[derive(Debug, thiserror::Error)]
#[error("the managed vector store lookup failed")]
pub struct VectorStoreRowsFailure {
    #[source]
    pub source: Box<AuthReadError<RowError>>,
}

#[async_trait]
pub trait ManagedVectorStoreCache: Send + Sync {
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError>;
    fn management_ttl(&self) -> f64;
    async fn write(
        &self,
        key: &str,
        row: &ManagedVectorStoresTable,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
}

#[async_trait]
pub trait ManagedVectorStoreRows: Send + Sync {
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure>;
}

#[async_trait]
impl<T> ManagedVectorStoreCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }

    async fn write(
        &self,
        key: &str,
        row: &ManagedVectorStoresTable,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(key), row, options).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManagedVectorStoreLookupError {
    #[error("the managed vector store cache read failed")]
    Cache(#[source] Box<PayloadError>),
    #[error("the managed vector store cache write failed")]
    CacheWrite(#[source] Box<AuthCacheError>),
    #[error("the managed vector store repository lookup failed")]
    Rows(#[source] Box<VectorStoreRowsFailure>),
    #[error("the stored managed vector store row is not a valid row")]
    Record(#[source] Box<serde_json::Error>),
}

// a projection that is not a nonempty object falls back to the iterable form, and an empty iterable skips the row
pub fn project_row(
    row: &VectorStoreRow,
) -> Result<Option<ManagedVectorStoresTable>, ManagedVectorStoreLookupError> {
    let projection = row.model_dump.as_ref().or(row.dict.as_ref());
    let fields = match projection {
        Some(JsonData::Object(fields)) if !fields.is_empty() => fields.clone(),
        _ if !row.fields.is_empty() => row.fields.clone(),
        _ => return Ok(None),
    };
    let text = serde_json::to_string(&JsonData::Object(fields))
        .map_err(|source| ManagedVectorStoreLookupError::Record(Box::new(source)))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| ManagedVectorStoreLookupError::Record(Box::new(source)))
}

pub async fn vector_store_rows_by_uuids(
    uuids: &[String],
    database: bool,
    cache: &dyn ManagedVectorStoreCache,
    rows: &dyn ManagedVectorStoreRows,
) -> Result<Vec<ManagedVectorStoresTable>, ManagedVectorStoreLookupError> {
    if uuids.is_empty() || !database {
        return Ok(Vec::new());
    }
    let (mut found, mut misses) = (Vec::new(), Vec::new());
    for uuid in uuids {
        let cached = cache
            .read(&format!("{VECTOR_STORE_CACHE_PREFIX}{uuid}"))
            .await
            .map_err(|source| ManagedVectorStoreLookupError::Cache(Box::new(source)))?;
        match cached {
            Some(row) => found.push(row),
            None => misses.push(uuid.clone()),
        }
    }
    if misses.is_empty() {
        return Ok(found);
    }
    let fetched = rows
        .by_uuids(ManagedRowsQuery {
            take: i64::try_from(misses.len()).unwrap_or(i64::MAX),
            uuids: misses,
        })
        .await
        .map_err(|source| ManagedVectorStoreLookupError::Rows(Box::new(source)))?;
    for raw in fetched {
        let Some(row) = project_row(&raw)? else {
            continue;
        };
        let key = format!("{VECTOR_STORE_CACHE_PREFIX}{}", row.vector_store_id);
        cache
            .write(&key, &row, cache.management_ttl())
            .await
            .map_err(|source| ManagedVectorStoreLookupError::CacheWrite(Box::new(source)))?;
        found.push(row);
    }
    Ok(found)
}
