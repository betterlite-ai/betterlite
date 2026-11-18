mod store;

mod tests;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheError};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::config::ConfigRow;
use betterlite_db::table_ops::TableOpsError;
use betterlite_db::table_ops::rows::RowError;
use betterlite_models::decisions::Json;

pub use store::{ConfigParamStore, TableConfigParamStore};

pub const CONFIG_CACHE_PREFIX: &str = "litellm_config:param:";
pub const MISS_MARKER: &str = "__litellm_config_param_miss__";
pub const USER_ROW_KEY_SUFFIX: &str = "_user_api_key_user_id";
pub const USER_ROW_TTL_SECONDS: f64 = 600.0;

#[derive(Debug, thiserror::Error)]
pub enum ConfigParamCacheError<ClientError = Infallible>
where
    ClientError: Error + Send + Sync + 'static,
{
    #[error("the config param read failed")]
    Database {
        #[source]
        source: DbError<RowError>,
    },
    #[error("the config param table read failed")]
    Table {
        #[source]
        source: TableOpsError<ClientError>,
    },
    #[error("the config param cache failed")]
    Cache {
        #[source]
        source: DualCacheError,
    },
    #[error("the cached config param row cannot be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

impl<ClientError> ConfigParamCacheError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    // B8379 reads the database fault off the error, so an outage stays distinguishable from a bad row
    #[must_use]
    pub fn database_source(&self) -> Option<&DbError<RowError>> {
        match self {
            Self::Database { source } => Some(source),
            Self::Table { source } => source.database_source(),
            Self::Cache { .. } | Self::Decode { .. } => None,
        }
    }
}

impl<ClientError> From<TableOpsError<ClientError>> for ConfigParamCacheError<ClientError>
where
    ClientError: Error + Send + Sync + 'static,
{
    fn from(source: TableOpsError<ClientError>) -> Self {
        match source {
            TableOpsError::Database { source } => Self::Database { source },
            TableOpsError::NoDbConnected => Self::Database {
                source: DbError::NoDbConnected,
            },
            other => Self::Table { source: other },
        }
    }
}

// LiteLLM's publish_config_param_change is best effort and returns nothing, so a Redis outage never fails a write
pub trait ConfigChangePublisher: Send + Sync {
    async fn publish_param_change(&self, param_name: &str);
}

pub struct ConfigParamCache<'cache, Memory: MemoryCacheOps> {
    pub cache: &'cache DualCache<Memory>,
    pub ttl_seconds: f64,
}

impl<Memory: MemoryCacheOps> ConfigParamCache<'_, Memory> {
    pub async fn get<Store>(
        &self,
        store: &Store,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, ConfigParamCacheError<Store::ClientError>>
    where
        Store: ConfigParamStore + ?Sized,
    {
        let key = format!("{CONFIG_CACHE_PREFIX}{param_name}");
        if let Some(cached) = self.cache.async_get_cache(&key, false).await {
            return unpack_row(&cached);
        }
        let row = store.find_param(param_name).await?;
        let value = match &row {
            Some(row) => pack_row(row)?,
            None => Json::Text(MISS_MARKER.to_owned()),
        };
        self.cache
            .async_set_cache(
                &key,
                &value,
                CacheOptions {
                    ttl: Some(self.ttl_seconds),
                    local_only: false,
                    refresh_ttl: false,
                },
            )
            .await;
        Ok(row)
    }

    pub async fn evict(&self, param_name: &str) -> Result<(), ConfigParamCacheError> {
        let key = format!("{CONFIG_CACHE_PREFIX}{param_name}");
        self.cache
            .async_delete_cache(&key)
            .await
            .map_err(|source| ConfigParamCacheError::Cache { source })
    }

    pub async fn invalidate<Publisher>(
        &self,
        param_name: &str,
        publisher: &Publisher,
    ) -> Result<(), ConfigParamCacheError>
    where
        Publisher: ConfigChangePublisher + ?Sized,
    {
        self.evict(param_name).await?;
        publisher.publish_param_change(param_name).await;
        Ok(())
    }

    // a failed batch read leaves every name uncached, so each one falls back to its own query
    pub async fn prefetch<Store>(&self, store: &Store, param_names: &[String])
    where
        Store: ConfigParamStore + ?Sized,
    {
        if param_names.is_empty() {
            return;
        }
        let rows = match store.find_params(param_names).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "prefetch_config_params failed, falling through to per-param queries"
                );
                return;
            }
        };
        let by_name: BTreeMap<&str, &ConfigRow> = rows
            .iter()
            .map(|row| (row.param_name.as_str(), row))
            .collect();
        for param_name in param_names {
            let value = match by_name.get(param_name.as_str()) {
                Some(row) => match pack_row(row) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::debug!(error = %error, "a prefetched config param cannot be cached");
                        continue;
                    }
                },
                None => Json::Text(MISS_MARKER.to_owned()),
            };
            let key = format!("{CONFIG_CACHE_PREFIX}{param_name}");
            self.cache
                .async_set_cache(
                    &key,
                    &value,
                    CacheOptions {
                        ttl: Some(self.ttl_seconds),
                        local_only: false,
                        refresh_ttl: false,
                    },
                )
                .await;
        }
    }
}

pub async fn cache_user_row<Memory, Store>(
    user_id: &str,
    cache: &ConfigParamCache<'_, Memory>,
    store: &Store,
) -> Result<(), ConfigParamCacheError<Store::ClientError>>
where
    Memory: MemoryCacheOps,
    Store: ConfigParamStore + ?Sized,
{
    let key = format!("{user_id}{USER_ROW_KEY_SUFFIX}");
    if cache.cache.get_cache(&key, false).is_some() {
        return Ok(());
    }
    let Some(row) = store.user_row(user_id).await? else {
        return Ok(());
    };
    let text =
        serde_json::to_string(&row).map_err(|source| ConfigParamCacheError::Decode { source })?;
    cache.cache.set_cache(
        &key,
        &Json::Text(text),
        CacheOptions {
            ttl: Some(USER_ROW_TTL_SECONDS),
            local_only: false,
            refresh_ttl: false,
        },
    );
    Ok(())
}

fn pack_row<ClientError>(row: &ConfigRow) -> Result<Json, ConfigParamCacheError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let wire =
        serde_json::to_value(row).map_err(|source| ConfigParamCacheError::Decode { source })?;
    serde_json::from_value(wire).map_err(|source| ConfigParamCacheError::Decode { source })
}

fn unpack_row<ClientError>(
    cached: &Json,
) -> Result<Option<ConfigRow>, ConfigParamCacheError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    match cached {
        Json::Text(text) if text == MISS_MARKER => Ok(None),
        Json::Object(_) => {
            let wire = serde_json::to_value(cached)
                .map_err(|source| ConfigParamCacheError::Decode { source })?;
            let row = serde_json::from_value(wire)
                .map_err(|source| ConfigParamCacheError::Decode { source })?;
            Ok(Some(row))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Ok(None),
    }
}
