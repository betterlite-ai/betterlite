use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_context::local_ttl_cache::CacheValue;
use tokio::select;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::{debug, warn};

use super::clock::BackoffClock;
use super::decode::decode_invalidation;
use super::endpoint::{AuthPubSub, AuthPubSubSubscription as _, PubSubMessage, TransportError};
use super::eviction::CacheError;
use super::message::{self, Invalidation};
use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;

pub const POLL_TIMEOUT: Duration = Duration::from_secs(1);
pub const BACKOFF_INITIAL: Duration = Duration::from_secs(5);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Session {
    pub tasks: JoinSet<()>,
    pub stop: Option<watch::Sender<bool>>,
}

pub struct CacheInvalidationSubscriber<E, C, T>
where
    T: AuthCacheTier,
{
    pub endpoint: Arc<E>,
    pub clock: C,
    pub user_cache: Arc<AuthCache<T>>,
    pub additional: Vec<Arc<T::Memory>>,
    pub session: Mutex<Session>,
}

enum Failure {
    Transport(TransportError),
    Cache(CacheError),
}

impl Failure {
    fn message(&self) -> String {
        match self {
            Self::Transport(error) => error.message(),
            Self::Cache(error) => error.to_string(),
        }
    }
}

enum Attempt<S> {
    Subscribed(S),
    Retry,
    Stop,
}

impl<E, C, T> CacheInvalidationSubscriber<E, C, T>
where
    E: AuthPubSub + 'static,
    C: BackoffClock + 'static,
    T: AuthCacheTier + 'static,
    T::Memory: 'static,
{
    pub fn start(self: &Arc<Self>) {
        let mut session = lock(&self.session);
        if session.stop.is_some() {
            return;
        }
        let (sender, receiver) = watch::channel(false);
        session.tasks.spawn(Arc::clone(self).run(receiver));
        session.stop = Some(sender);
    }

    pub async fn stop(&self) {
        let mut tasks = {
            let mut session = lock(&self.session);
            let Some(sender) = session.stop.take() else {
                return;
            };
            let tasks = std::mem::take(&mut session.tasks);
            let _sent = sender.send(true);
            tasks
        };
        // the signal ends the loop at its next await, and joining lets it close the pubsub first
        while tasks.join_next().await.is_some() {}
    }

    async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut backoff = BACKOFF_INITIAL;
        loop {
            let mut subscription = match self.subscribe_once(&mut backoff, &mut shutdown).await {
                Attempt::Subscribed(subscription) => subscription,
                Attempt::Retry => continue,
                Attempt::Stop => return,
            };
            backoff = BACKOFF_INITIAL;
            let failure = self.consume(&mut subscription, &mut shutdown).await;
            if let Err(error) = subscription.close().await {
                let message = error.message();
                debug!(
                    subject = message.as_str(),
                    "auth cache invalidation pubsub close failed: {message}"
                );
            }
            let Some(reported) = failure else {
                return;
            };
            let message = reported.message();
            if self.pause(&message, &mut backoff, &mut shutdown).await {
                return;
            }
        }
    }

    async fn subscribe_once(
        &self,
        backoff: &mut Duration,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Attempt<E::Subscription> {
        let subscribed = select! {
            biased;
            _ = shutdown.changed() => return Attempt::Stop,
            outcome = self.endpoint.subscribe() => outcome,
        };
        match subscribed {
            Ok(subscription) => Attempt::Subscribed(subscription),
            Err(error) => {
                let message = error.message();
                let stopping = self.pause(&message, backoff, shutdown).await;
                if stopping {
                    return Attempt::Stop;
                }
                Attempt::Retry
            }
        }
    }

    async fn pause(
        &self,
        message: &str,
        backoff: &mut Duration,
        shutdown: &mut watch::Receiver<bool>,
    ) -> bool {
        warn!(
            subject = message,
            backoff_seconds = backoff.as_secs_f64(),
            "auth cache invalidation subscriber redis error: {message}; reconnecting in {:.0}s",
            backoff.as_secs_f64()
        );
        let stopped = select! {
            biased;
            _ = shutdown.changed() => true,
            () = self.clock.sleep(*backoff) => false,
        };
        *backoff = backoff.saturating_mul(2).min(BACKOFF_MAX);
        stopped
    }

    async fn consume(
        &self,
        subscription: &mut E::Subscription,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Option<Failure> {
        loop {
            let outcome = select! {
                biased;
                _ = shutdown.changed() => return None,
                outcome = subscription.next_message(POLL_TIMEOUT) => outcome,
            };
            let received = match outcome {
                Ok(None) => continue,
                Ok(Some(received)) => received,
                Err(error) => return Some(Failure::Transport(error)),
            };
            if let Err(error) = self.apply(&received) {
                return Some(Failure::Cache(error));
            }
        }
    }

    fn apply(&self, received: &PubSubMessage) -> Result<(), CacheError> {
        let Some(data) = received.data.as_deref() else {
            return Ok(());
        };
        let Ok(typed) = decode_invalidation(data) else {
            return Ok(());
        };
        let Some(invalidation) = message::read(&typed) else {
            return Ok(());
        };
        let Some(new_value) = invalidation.new_value else {
            self.drop_entry(&invalidation.cache_key)?;
            return Ok(());
        };
        if !new_value.is_finite() {
            warn!(
                subject = invalidation.cache_key.as_str(),
                "auth cache invalidation set for {} carries a number a counter cannot hold; dropped",
                invalidation.cache_key
            );
            return Ok(());
        }
        for cache in &self.additional {
            write(cache.as_ref(), &invalidation, new_value)?;
        }
        Ok(())
    }

    fn drop_entry(&self, cache_key: &str) -> Result<(), CacheError> {
        drop_from(
            self.user_cache.in_memory_cache_for(cache_key).as_ref(),
            cache_key,
        )?;
        for cache in &self.additional {
            drop_from(cache.as_ref(), cache_key)?;
        }
        Ok(())
    }
}

fn write<M>(cache: &M, invalidation: &Invalidation, new_value: f64) -> Result<(), CacheError>
where
    M: MemoryCacheOps,
{
    cache
        .set_cache(
            &invalidation.cache_key,
            CacheValue::Float(new_value),
            invalidation.ttl,
        )
        .map_err(|source| CacheError::Failed {
            cache_key: invalidation.cache_key.clone(),
            source,
        })
}

fn drop_from<M>(cache: &M, cache_key: &str) -> Result<(), CacheError>
where
    M: MemoryCacheOps,
{
    cache
        .delete_cache(cache_key)
        .map_err(|source| CacheError::Failed {
            cache_key: cache_key.to_owned(),
            source,
        })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
