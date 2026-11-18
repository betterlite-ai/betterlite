use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::Semaphore;
use tokio::task::{JoinSet, yield_now};
use tracing::warn;

use super::endpoint::AuthPubSub;
use super::eviction::LocalEviction;
use super::message::{self, Invalidation};

pub const MAX_PENDING_PUBLISHES: usize = 1024;
pub const MAX_IN_FLIGHT_PUBLISHES: usize = 16;

pub struct PublishTasks {
    pending: Mutex<JoinSet<()>>,
    in_flight: Arc<Semaphore>,
}

impl Default for PublishTasks {
    fn default() -> Self {
        Self {
            pending: Mutex::new(JoinSet::new()),
            in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT_PUBLISHES)),
        }
    }
}

impl PublishTasks {
    pub async fn shutdown(&self) {
        let mut pending = std::mem::take(&mut *lock(&self.pending));
        pending.abort_all();
        while pending.join_next().await.is_some() {}
    }
}

pub async fn publish_invalidation<E>(
    tasks: &PublishTasks,
    endpoint: Option<&Arc<E>>,
    invalidation: &Invalidation,
) where
    E: AuthPubSub + 'static,
{
    let Some(endpoint) = endpoint else {
        return;
    };
    let cache_key = invalidation.cache_key.clone();
    {
        let mut pending = lock(&tasks.pending);
        while pending.try_join_next().is_some() {}
        if pending.len() >= MAX_PENDING_PUBLISHES {
            drop(pending);
            warn!(
                subject = cache_key.as_str(),
                pending = MAX_PENDING_PUBLISHES,
                "auth cache invalidation publish for {cache_key} dropped: {MAX_PENDING_PUBLISHES} publishes already waiting on redis; other workers keep their cached copy until its TTL expires"
            );
            return;
        }
        let message = message::encode(invalidation);
        let endpoint = Arc::clone(endpoint);
        let in_flight = Arc::clone(&tasks.in_flight);
        pending.spawn(async move {
            let permit = in_flight.acquire().await;
            let Ok(_held) = permit else {
                warn!(
                    subject = cache_key.as_str(),
                    "auth cache invalidation publish for {cache_key} found the in-flight limit closed"
                );
                return;
            };
            if let Err(error) = endpoint.publish(&message).await {
                warn!(
                    subject = cache_key.as_str(),
                    error = error.message(),
                    "auth cache invalidation publish for {cache_key} failed: {}",
                    error.message()
                );
            }
        });
    }
    yield_now().await;
}

pub async fn evict_and_broadcast<E>(
    cache_keys: &[String],
    eviction: &dyn LocalEviction,
    tasks: &PublishTasks,
    endpoint: Option<&Arc<E>>,
) where
    E: AuthPubSub + 'static,
{
    for cache_key in cache_keys {
        let invalidation = Invalidation {
            cache_key: cache_key.clone(),
            new_value: None,
            ttl: None,
        };
        if let Err(error) = eviction.apply(invalidation.clone()) {
            warn!(
                subject = cache_key.as_str(),
                error = %error,
                "Failed to evict cached entry {cache_key}; a stale object may be served until its TTL expires: {error}"
            );
        }
        publish_invalidation(tasks, endpoint, &invalidation).await;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
