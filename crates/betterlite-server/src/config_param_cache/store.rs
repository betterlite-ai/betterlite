use betterlite_db::table_ops::generic::GenericReads as _;
use betterlite_db::table_ops::reads::configs::ConfigReads as _;
use betterlite_db::table_ops::reads::users::UserReads as _;
use std::error::Error;
use std::future::Future;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_db::database_replica::client::DatabaseClient;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::table_ops::access::TablePoolAccess;
use betterlite_db::table_ops::generic::ConfigFilter;
use betterlite_db::table_ops::rows::ConfigParamRow;
use betterlite_db::table_ops::{ConfigParamInvalidator, InvalidateFuture, TableOps, TableOpsError};

use super::{ConfigChangePublisher, ConfigParamCache};

// what LiteLLM reaches through PrismaClient.get_generic_data, the config find_many and get_data(user_id)
pub trait ConfigParamStore: Sync {
    type ClientError: Error + Send + Sync + 'static;

    fn find_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<ConfigParamRow>, TableOpsError<Self::ClientError>>> + Send;

    fn find_params(
        &self,
        param_names: &[String],
    ) -> impl Future<Output = Result<Vec<ConfigParamRow>, TableOpsError<Self::ClientError>>> + Send;

    fn user_row(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, TableOpsError<Self::ClientError>>> + Send;
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
    ) -> Result<Option<ConfigParamRow>, TableOpsError<Self::ClientError>> {
        self.operations
            .generic_row(ConfigFilter::ParamName(param_name))
            .await
    }

    async fn find_params(
        &self,
        param_names: &[String],
    ) -> Result<Vec<ConfigParamRow>, TableOpsError<Self::ClientError>> {
        self.operations.configs_named(param_names).await
    }

    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Self::ClientError>> {
        self.operations.user_unique(user_id).await
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
