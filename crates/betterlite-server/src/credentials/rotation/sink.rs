use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_redis::config_sync::publish::publish_config_change;
use betterlite_redis::config_sync::tables::SyncedTable;
use betterlite_redis::redis_cache::RedisCache;

// The one message _rotate_master_key sends after its model transaction committed.
pub trait ModelTableInvalidation: Sync {
    type Error: Error + Send + 'static;

    fn model_table_changed(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// object_type="litellm_proxymodeltable" over the coordination Redis; publish_config_change is best effort, so this sink cannot fail.
pub struct RedisModelTableInvalidation<'cache> {
    pub redis: Option<&'cache RedisCache>,
}

impl ModelTableInvalidation for RedisModelTableInvalidation<'_> {
    type Error = Infallible;

    async fn model_table_changed(&self) -> Result<(), Infallible> {
        publish_config_change(self.redis, SyncedTable::ProxyModelTable.name()).await;
        Ok(())
    }
}
