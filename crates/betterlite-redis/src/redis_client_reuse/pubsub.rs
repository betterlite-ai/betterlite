use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use redis::aio::PubSub;
use redis::cmd;
use tokio::runtime::Handle;
use tokio::time;

use super::error::ClientReuseError;
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::cluster::ClusterClient;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_live_client::{LiveAsyncClient, LiveClientError, LiveClientFactory, LiveTarget};

pub struct LivePubSubClient {
    pub(crate) client: LiveAsyncClient,
    pub(crate) record: RedisClient,
    pub(crate) factory: Arc<LiveClientFactory>,
}

pub struct LiveSubscription {
    pubsub: PubSub,
}

pub struct LiveMessage {
    pub channel: String,
    pub payload: Vec<u8>,
}

impl LivePubSubClient {
    pub async fn publish(&self, channel: &str, message: &str) -> Result<i64, ClientReuseError> {
        Ok(self
            .client
            .query(cmd("PUBLISH").arg(channel).arg(message))
            .await?)
    }

    pub async fn subscribe(
        &self,
        channels: &[String],
    ) -> Result<LiveSubscription, ClientReuseError> {
        let factory = Arc::clone(&self.factory);
        let record = self.record.clone();
        let endpoint = Handle::current()
            .spawn_blocking(move || factory.blocking_endpoint(&record))
            .await
            .map_err(LiveClientError::Join)??;
        let LiveTarget::Standalone(client) = endpoint.target else {
            return Err(ClientReuseError::ClusterPubSub);
        };
        let mut pubsub = time::timeout(endpoint.connect_timeout, client.get_async_pubsub())
            .await
            .map_err(ClientReuseError::Timeout)?
            .map_err(LiveClientError::Redis)?;
        time::timeout(endpoint.response_timeout, pubsub.subscribe(channels))
            .await
            .map_err(ClientReuseError::Timeout)?
            .map_err(LiveClientError::Redis)?;
        Ok(LiveSubscription { pubsub })
    }
}

impl LiveSubscription {
    pub async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<LiveMessage>, ClientReuseError> {
        let mut messages = self.pubsub.on_message();
        match time::timeout(timeout, messages.next()).await {
            Ok(Some(message)) => Ok(Some(LiveMessage {
                channel: message.get_channel_name().to_owned(),
                payload: message.get_payload_bytes().to_vec(),
            })),
            Ok(None) | Err(_) => Ok(None),
        }
    }
}

// litellm.caching.redis_cache._cluster_node_pubsub_client: one plain connection on the first startup node.
pub fn cluster_node_connection(
    cluster: &ClusterClient,
) -> Result<ConnectionFields, ClientReuseError> {
    let node = cluster
        .startup_nodes
        .first()
        .ok_or(ClientReuseError::NoPubSubNode)?;
    let mut passthrough = cluster.passthrough.clone();
    passthrough.remove("read_from_replicas");
    Ok(ConnectionFields {
        host: Some(node.host.clone()),
        port: Some(node.port),
        db: None,
        username: cluster.username.clone(),
        password: cluster.password.clone(),
        ssl: cluster.ssl,
        socket_timeout: cluster.socket_timeout,
        socket_connect_timeout: cluster.socket_connect_timeout,
        socket_keepalive: cluster.socket_keepalive,
        max_connections: cluster.max_connections,
        health_check_interval: cluster.health_check_interval,
        credential_provider: cluster.credential_provider,
        credential_provider_settings: cluster.credential_provider_settings.clone(),
        ssl_password: cluster.ssl_password.clone(),
        passthrough,
    })
}
