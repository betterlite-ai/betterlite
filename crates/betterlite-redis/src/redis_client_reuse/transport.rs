use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;
use tokio::task::yield_now;

use super::cache_key::async_client_cache_key;
use super::error::ClientReuseError;
use super::pubsub::{LivePubSubClient, cluster_node_connection};
use super::store::RedisClientStore;
use super::sync_client::LiveSyncClient;
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::dispatch::build_client;
use crate::redis_client_factories::pool::PoolConfig;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisConnectionSettings};
use crate::redis_live_client::{LiveAsyncClient, LiveClientFactory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedisCacheFamily {
    Standalone,
    Cluster,
}

pub struct RedisTransport {
    pub family: RedisCacheFamily,
    pub cache_key: String,
    pub sync_client: LiveSyncClient,
    pub async_record: RedisClient,
    pub factory: Arc<LiveClientFactory>,
    pub store: Arc<RedisClientStore>,
    pub pool: Option<PoolConfig>,
    pub cluster_client: OnceCell<LiveAsyncClient>,
    pub settings: RedisConnectionSettings,
}

impl RedisTransport {
    pub fn new(
        settings: RedisConnectionSettings,
        family: RedisCacheFamily,
        factory: Arc<LiveClientFactory>,
        store: Arc<RedisClientStore>,
        pool: Option<PoolConfig>,
    ) -> Result<Self, ClientReuseError> {
        let cache_key = async_client_cache_key(&settings);
        let async_record = build_client(&settings, ClientKind::Async)?;
        let sync_record = build_client(&settings, ClientKind::Sync)?;
        Ok(Self {
            family,
            cache_key,
            sync_client: LiveSyncClient {
                factory: Arc::clone(&factory),
                client: sync_record,
                connection: Mutex::new(None),
            },
            async_record,
            factory,
            store,
            pool,
            cluster_client: OnceCell::new(),
            settings,
        })
    }

    pub async fn async_client(&self) -> Result<LiveAsyncClient, ClientReuseError> {
        match self.family {
            RedisCacheFamily::Standalone => {
                self.stored_client(&self.cache_key, &self.async_record)
                    .await
            }
            RedisCacheFamily::Cluster => {
                if let Some(client) = self.cluster_client.get() {
                    return Ok(client.clone());
                }
                let opened = self
                    .factory
                    .open_async(&self.async_record, self.pool.as_ref())
                    .await?;
                if !matches!(&opened, LiveAsyncClient::Cluster(_)) {
                    return Ok(opened);
                }
                let held = self.cluster_client.get_or_init(|| async { opened }).await;
                Ok(held.clone())
            }
        }
    }

    pub fn mget(&self, keys: &[String]) -> Result<Vec<redis::Value>, ClientReuseError> {
        let Some(command) = mget_command(keys) else {
            return Ok(Vec::new());
        };
        self.sync_client.query(&command)
    }

    pub async fn async_mget(&self, keys: &[String]) -> Result<Vec<redis::Value>, ClientReuseError> {
        let Some(command) = mget_command(keys) else {
            return Ok(Vec::new());
        };
        let client = self.async_client().await?;
        Ok(client.query(&command).await?)
    }

    pub async fn pubsub_client(&self) -> Result<LivePubSubClient, ClientReuseError> {
        let client = self.async_client().await?;
        let record = match (&client, &self.async_record) {
            (LiveAsyncClient::Cluster(_), RedisClient::Cluster(cluster_record)) => {
                RedisClient::Direct(cluster_node_connection(cluster_record)?)
            }
            (LiveAsyncClient::Standalone(_) | LiveAsyncClient::Sentinel(_), _)
            | (
                // open_async builds a Cluster client only from a Cluster record, so this arm is unreachable
                LiveAsyncClient::Cluster(_),
                RedisClient::Direct(_) | RedisClient::Url(_) | RedisClient::Sentinel(_),
            ) => {
                return Ok(LivePubSubClient {
                    client,
                    record: self.async_record.clone(),
                    factory: Arc::clone(&self.factory),
                });
            }
        };
        let key = format!("{}-pubsub", self.cache_key);
        let opened = self.stored_client(&key, &record).await?;
        Ok(LivePubSubClient {
            client: opened,
            record,
            factory: Arc::clone(&self.factory),
        })
    }

    async fn stored_client(
        &self,
        key: &str,
        record: &RedisClient,
    ) -> Result<LiveAsyncClient, ClientReuseError> {
        self.store
            .get_or_open(key, &self.factory, record, self.pool.as_ref())
            .await
    }

    pub async fn close_pubsub_client(&self) {
        self.store.remove(&format!("{}-pubsub", self.cache_key));
        // the dropped client's driver task needs one poll to observe the close
        yield_now().await;
    }
}

// litellm uses mget_nonatomic on a cluster and a plain mget elsewhere; both send nothing for an empty list.
fn mget_command(keys: &[String]) -> Option<redis::Cmd> {
    if keys.is_empty() {
        return None;
    }
    let mut command = redis::cmd("MGET");
    command.arg(keys);
    Some(command)
}
