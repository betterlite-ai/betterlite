use std::error::Error;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_db::database_replica::client::DatabaseClient;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::config::ConfigRow;
use betterlite_db::table_ops::access::TablePoolAccess;
use betterlite_db::table_ops::{ConfigParamInvalidator, InvalidateFuture, TableOps};

use super::{ConfigChangePublisher, ConfigParamCache, ConfigParamCacheError};

// what LiteLLM reaches through PrismaClient.get_generic_data, the config find_many and get_data(user_id)
pub trait ConfigParamStore {
    type ClientError: Error + Send + Sync + 'static;

    async fn find_param(
        &self,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, ConfigParamCacheError<Self::ClientError>>;

    async fn find_params(
        &self,
        param_names: &[String],
    ) -> Result<Vec<ConfigRow>, ConfigParamCacheError<Self::ClientError>>;

    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, ConfigParamCacheError<Self::ClientError>>;
}

pub struct TableConfigParamStore<'service, 'ops, Client>
where
    Client: DatabaseClient,
{
    pub operations: &'service TableOps<'ops, Client>,
}

impl<Client> ConfigParamStore for TableConfigParamStore<'_, '_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    type ClientError = Client::Error;

    async fn find_param(
        &self,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, ConfigParamCacheError<Self::ClientError>> {
        self.operations
            .generic_config(param_name)
            .await
            .map_err(ConfigParamCacheError::from)
    }

    async fn find_params(
        &self,
        param_names: &[String],
    ) -> Result<Vec<ConfigRow>, ConfigParamCacheError<Self::ClientError>> {
        self.operations
            .configs_named(param_names)
            .await
            .map_err(ConfigParamCacheError::from)
    }

    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, ConfigParamCacheError<Self::ClientError>> {
        self.operations
            .user_unique(user_id)
            .await
            .map_err(ConfigParamCacheError::from)
    }
}

// insert_data awaits invalidate_config_param after each config write: evict here, then publish the change
pub struct ConfigParamCacheInvalidator<'view, 'cache, Memory, Publisher>
where
    Memory: MemoryCacheOps,
{
    pub cache: &'view ConfigParamCache<'cache, Memory>,
    pub publisher: &'view Publisher,
}

impl<Memory, Publisher> ConfigParamInvalidator
    for ConfigParamCacheInvalidator<'_, '_, Memory, Publisher>
where
    Memory: MemoryCacheOps,
    Publisher: ConfigChangePublisher,
{
    fn invalidate_param<'call>(&'call self, param_name: &'call str) -> InvalidateFuture<'call> {
        Box::pin(async move {
            if let Err(error) = self.cache.evict(param_name).await {
                tracing::warn!(
                    error = %error,
                    param_name,
                    "the written config param could not be evicted from the cache"
                );
            }
            self.publisher.publish_param_change(param_name).await;
        })
    }
}
