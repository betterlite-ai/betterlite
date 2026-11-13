use std::sync::Arc;
use std::time::Duration;

use super::channel::config_sync_channel;
use super::endpoint::{ConfigSyncMessage, ConfigSyncPubSubClient, ConfigSyncSubscription};
use super::error::ConfigSyncError;
use crate::redis_cache::RedisCache;
use crate::redis_client_reuse::pubsub::LiveSubscription;

pub struct RedisConfigSyncClient {
    pub cache: Arc<RedisCache>,
}

pub struct RedisConfigSyncSubscription {
    pub subscription: Option<LiveSubscription>,
}

impl ConfigSyncPubSubClient for RedisConfigSyncClient {
    type Subscription = RedisConfigSyncSubscription;

    async fn subscribe(&self) -> Result<Self::Subscription, ConfigSyncError> {
        let channel = config_sync_channel(self.cache.namespace.as_deref());
        let client = self.cache.transport.pubsub_client().await?;
        let subscription = client.subscribe(&[channel]).await?;
        Ok(RedisConfigSyncSubscription {
            subscription: Some(subscription),
        })
    }
}

impl ConfigSyncSubscription for RedisConfigSyncSubscription {
    async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<ConfigSyncMessage>, ConfigSyncError> {
        let Some(subscription) = self.subscription.as_mut() else {
            return Ok(None);
        };
        Ok(subscription
            .next_message(timeout)
            .await?
            .map(|message| ConfigSyncMessage {
                channel: message.channel,
                payload: message.payload,
            }))
    }

    async fn close(&mut self) {
        self.subscription = None;
    }
}
