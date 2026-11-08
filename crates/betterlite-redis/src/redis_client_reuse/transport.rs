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
                self.store
                    .get_or_open(
                        &self.cache_key,
                        &self.factory,
                        &self.async_record,
                        self.pool.as_ref(),
                    )
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
                let held = self
                    .cluster_client
                    .get_or_try_init(|| async { Ok::<LiveAsyncClient, ClientReuseError>(opened) })
                    .await?;
                Ok(held.clone())
            }
        }
    }

    pub fn mget(&self, keys: &[String]) -> Result<Vec<redis::Value>, ClientReuseError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut command = redis::cmd("MGET");
        command.arg(keys);
        self.sync_client.query(&command)
    }

    pub async fn async_mget(&self, keys: &[String]) -> Result<Vec<redis::Value>, ClientReuseError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut command = redis::cmd("MGET");
        command.arg(keys);
        let client = self.async_client().await?;
        Ok(client.query(&command).await?)
    }

    pub async fn pubsub_client(&self) -> Result<LivePubSubClient, ClientReuseError> {
        let client = self.async_client().await?;
        let cluster = match (&client, &self.async_record) {
            (LiveAsyncClient::Cluster(_), RedisClient::Cluster(cluster_record)) => {
                Some(cluster_record)
            }
            _ => None,
        };
        let Some(cluster_record) = cluster else {
            return Ok(LivePubSubClient {
                client,
                record: self.async_record.clone(),
                factory: Arc::clone(&self.factory),
            });
        };
        let record = RedisClient::Direct(cluster_node_connection(cluster_record)?);
        let key = format!("{}-pubsub", self.cache_key);
        let opened = self
            .store
            .get_or_open(&key, &self.factory, &record, self.pool.as_ref())
            .await?;
        Ok(LivePubSubClient {
            client: opened,
            record,
            factory: Arc::clone(&self.factory),
        })
    }

    pub async fn close_pubsub_client(&self) {
        self.store.remove(&format!("{}-pubsub", self.cache_key));
        // the dropped client's driver task needs one poll to observe the close
        yield_now().await;
    }
}
