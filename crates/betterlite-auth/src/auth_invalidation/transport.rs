use std::sync::Arc;

use async_trait::async_trait;

use crate::auth_pubsub::endpoint::AuthPubSub;
use crate::auth_pubsub::eviction::LocalEviction;
use crate::auth_pubsub::message::Invalidation;
use crate::auth_pubsub::publish::{PublishTasks, evict_and_broadcast, publish_invalidation};

#[async_trait]
pub trait InvalidationTransport: Send + Sync {
    async fn publish(&self, cache_key: &str, new_value: Option<f64>, ttl: Option<f64>);
    async fn evict_and_broadcast(&self, cache_keys: &[String], local: &dyn LocalEviction);
}

pub struct PubSubTransport<E> {
    pub tasks: PublishTasks,
    pub endpoint: Option<Arc<E>>,
}

#[async_trait]
impl<E> InvalidationTransport for PubSubTransport<E>
where
    E: AuthPubSub + 'static,
{
    async fn publish(&self, cache_key: &str, new_value: Option<f64>, ttl: Option<f64>) {
        let invalidation = Invalidation {
            cache_key: cache_key.to_owned(),
            new_value,
            ttl,
        };
        publish_invalidation(&self.tasks, self.endpoint.as_ref(), &invalidation).await;
    }

    async fn evict_and_broadcast(&self, cache_keys: &[String], local: &dyn LocalEviction) {
        evict_and_broadcast(cache_keys, local, &self.tasks, self.endpoint.as_ref()).await;
    }
}
