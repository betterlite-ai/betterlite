use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::join_all;
use indexmap::IndexMap;
use indexmap::map::Entry;
use tokio::runtime::Handle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::prefetch::PrefetchSlot;
use super::{BackendKey, lock, registry};
use crate::batch::RedisBatch;
use crate::redis_cache::RedisCache;

// litellm.caching.redis_batch.POST_CALL_FLUSH_DEADLINE_SECONDS, kept as a Duration so it types the deadline at its boundary.
pub const POST_CALL_FLUSH_DEADLINE: Duration = Duration::from_secs(1);

pub struct RequestRedisBatches {
    pub post_call_deadline: Duration,
    pub batches: Mutex<IndexMap<BackendKey, Arc<RedisBatch>>>,
    pub post_call: Mutex<IndexMap<BackendKey, Arc<RedisBatch>>>,
    pub deadline: Mutex<Option<Arc<CancellationToken>>>,
    // consumers outside this crate arm and take their own typed slot values through lock()
    pub prefetched: Mutex<BTreeMap<String, PrefetchSlot>>,
    pub follow_ups: TaskTracker,
}

impl RequestRedisBatches {
    #[must_use]
    pub fn batch(&self, redis_cache: &Arc<RedisCache>) -> Arc<RedisBatch> {
        Self::get_or_create(
            &self.batches,
            redis_cache,
            "request_redis_batch",
            &self.follow_ups,
        )
    }

    #[must_use]
    pub fn post_call(self: &Arc<Self>, redis_cache: &Arc<RedisCache>) -> Arc<RedisBatch> {
        let batch = Self::get_or_create(
            &self.post_call,
            redis_cache,
            "post_call_redis_batch",
            &self.follow_ups,
        );
        self.arm_deadline();
        registry::apply(self, registry::Change::Register);
        batch
    }

    fn get_or_create(
        slots: &Mutex<IndexMap<BackendKey, Arc<RedisBatch>>>,
        redis_cache: &Arc<RedisCache>,
        name: &'static str,
        follow_ups: &TaskTracker,
    ) -> Arc<RedisBatch> {
        let key = (
            redis_cache.transport.cache_key.clone(),
            redis_cache.namespace.clone(),
        );
        let mut open = lock(slots);
        match open.entry(key) {
            Entry::Occupied(entry) => Arc::clone(entry.get()),
            Entry::Vacant(entry) => {
                let batch = Arc::new(RedisBatch::new(
                    Arc::clone(redis_cache),
                    name,
                    follow_ups.clone(),
                ));
                entry.insert(Arc::clone(&batch));
                batch
            }
        }
    }

    pub async fn flush_all(self: &Arc<Self>) {
        Self::flush_slots(&self.batches).await;
        let post_call_idle = {
            let post_call = lock(&self.post_call);
            post_call.values().all(|batch| batch.pending() == 0)
        };
        if post_call_idle {
            self.cancel_deadline();
            self.discard_if_idle();
        }
    }

    pub async fn flush_post_call(self: &Arc<Self>) {
        self.cancel_deadline();
        Self::flush_slots(&self.post_call).await;
        self.discard_if_idle();
    }

    async fn flush_slots(slots: &Mutex<IndexMap<BackendKey, Arc<RedisBatch>>>) {
        let pending = Self::pending(slots);
        let flushes = pending.iter().map(|batch| batch.flush());
        join_all(flushes).await;
    }

    fn pending(slots: &Mutex<IndexMap<BackendKey, Arc<RedisBatch>>>) -> Vec<Arc<RedisBatch>> {
        lock(slots)
            .values()
            .filter(|batch| batch.pending() > 0)
            .cloned()
            .collect()
    }

    fn cancel_deadline(&self) {
        let deadline = lock(&self.deadline).take();
        if let Some(token) = deadline {
            token.cancel();
        }
    }

    pub(super) async fn drain_follow_ups(&self) {
        self.follow_ups.close();
        self.follow_ups.wait().await;
    }

    fn arm_deadline(self: &Arc<Self>) {
        let token = Arc::new(CancellationToken::new());
        {
            let mut deadline = lock(&self.deadline);
            if deadline.is_some() {
                return;
            }
            *deadline = Some(Arc::clone(&token));
        };
        let Ok(runtime) = Handle::try_current() else {
            // nothing would fire the deadline without a runtime, so a later post_call arms it again
            drop(lock(&self.deadline).take());
            tracing::warn!(
                "request Redis batches need a Tokio runtime to flush post-call writes on the deadline"
            );
            return;
        };
        let owned = Arc::clone(self);
        self.follow_ups.spawn_on(
            async move { owned.flush_at_deadline(&token).await },
            &runtime,
        );
    }

    async fn flush_at_deadline(self: Arc<Self>, token: &Arc<CancellationToken>) {
        if timeout(self.post_call_deadline, token.cancelled())
            .await
            .is_ok()
        {
            return;
        }
        let expired = {
            let mut deadline = lock(&self.deadline);
            let current = deadline.take();
            match current {
                Some(armed) if Arc::ptr_eq(&armed, token) => true,
                other => {
                    *deadline = other;
                    false
                }
            }
        };
        if expired {
            Self::flush_slots(&self.post_call).await;
            self.discard_if_idle();
        }
    }

    fn discard_if_idle(self: &Arc<Self>) {
        let idle = {
            let post_call = lock(&self.post_call);
            post_call.values().all(|batch| batch.pending() == 0)
        };
        if idle {
            registry::apply(self, registry::Change::Discard);
        }
    }
}
