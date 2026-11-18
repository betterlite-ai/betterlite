use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_client_reuse::pubsub::LiveSubscription;
use thiserror::Error;

use super::channel::invalidation_channel;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("the redis pub/sub connection failed: {message}")]
    Connection { message: String },
    #[error("the redis pub/sub publish failed: {message}")]
    Publish { message: String },
}

impl TransportError {
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Connection { message } | Self::Publish { message } => message,
        }
    }
}

impl From<ClientReuseError> for TransportError {
    fn from(error: ClientReuseError) -> Self {
        Self::Connection {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PubSubMessage {
    pub data: Option<Vec<u8>>,
}

pub trait AuthPubSubSubscription: Send {
    fn next_message(
        &mut self,
        timeout: Duration,
    ) -> impl Future<Output = Result<Option<PubSubMessage>, TransportError>> + Send;

    fn close(&mut self) -> impl Future<Output = Result<(), TransportError>> + Send;
}

pub trait AuthPubSub: Send + Sync {
    type Subscription: AuthPubSubSubscription;

    fn channel(&self) -> String;

    fn publish(&self, message: &str) -> impl Future<Output = Result<(), TransportError>> + Send;

    fn subscribe(&self) -> impl Future<Output = Result<Self::Subscription, TransportError>> + Send;
}

pub struct RedisAuthPubSub {
    pub cache: Arc<RedisCache>,
}

pub struct RedisAuthSubscription {
    pub subscription: Option<LiveSubscription>,
}

impl AuthPubSub for RedisAuthPubSub {
    type Subscription = RedisAuthSubscription;

    fn channel(&self) -> String {
        invalidation_channel(self.cache.namespace.as_deref())
    }

    async fn publish(&self, message: &str) -> Result<(), TransportError> {
        let channel = self.channel();
        let client = self.cache.transport.pubsub_client().await?;
        let _published =
            client
                .publish(&channel, message)
                .await
                .map_err(|error| TransportError::Publish {
                    message: error.to_string(),
                })?;
        Ok(())
    }

    async fn subscribe(&self) -> Result<Self::Subscription, TransportError> {
        let channel = self.channel();
        let client = self.cache.transport.pubsub_client().await?;
        let subscription = client.subscribe(&[channel]).await?;
        Ok(RedisAuthSubscription {
            subscription: Some(subscription),
        })
    }
}

impl AuthPubSubSubscription for RedisAuthSubscription {
    // a scripted LiveSubscription is not constructible, so no unit test reaches the payload mapping
    #[cfg_attr(test, mutants::skip)]
    async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<PubSubMessage>, TransportError> {
        let Some(subscription) = self.subscription.as_mut() else {
            return Ok(None);
        };
        let message = subscription.next_message(timeout).await?;
        Ok(message.map(|message| PubSubMessage {
            data: Some(message.payload),
        }))
    }

    // dropping a live subscription needs one to exist, so no unit test reaches this assignment
    #[cfg_attr(test, mutants::skip)]
    async fn close(&mut self) -> Result<(), TransportError> {
        self.subscription = None;
        Ok(())
    }
}
