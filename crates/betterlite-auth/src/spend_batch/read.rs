use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::atomic::Ordering;

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::request_batch::lock;

use super::batch::{BatchPhase, SpendCounterBatch, SpendCounterValue};
use super::value::decimal_map;

impl SpendCounterBatch {
    // None means the caller reads Redis itself: the batch is closed, the key is unbound, or the fetch failed
    pub async fn read(&self, counter_key: &str) -> Option<SpendCounterValue> {
        let attempt = CounterRead {
            batch: self,
            counter_key,
        };
        if !attempt.settle().await {
            return None;
        }
        lock(&self.state)
            .loaded
            .get(counter_key)
            .copied()
            .map(SpendCounterValue::from)
    }

    async fn fetch(&self) {
        let _published = PublishOnDrop { batch: self };
        if self.request_batch.is_some() {
            self.declare_pending();
            self.collect_inflight().await;
        } else {
            self.fetch_pending().await;
        }
    }

    async fn collect_inflight(&self) {
        let results = {
            let mut state = lock(&self.state);
            mem::take(&mut state.inflight)
        };
        for result in results {
            match result.await {
                Ok(values) => self.merge_fetched(&values),
                Err(error) => tracing::debug!(
                    error = %error,
                    "a spend counter batch read fell back to a direct read"
                ),
            }
        }
    }

    async fn fetch_pending(&self) {
        let pending = {
            let mut state = lock(&self.state);
            let pending: BTreeSet<String> =
                state.keys.difference(&state.fetched).cloned().collect();
            state.fetched.extend(pending.iter().cloned());
            pending
        };
        if pending.is_empty() {
            return;
        }
        let keys: Vec<Option<&str>> = pending.iter().map(|key| Some(key.as_str())).collect();
        match self.redis_cache.async_batch_get_cache(&keys).await {
            Ok(values) => {
                let returned: BTreeMap<String, Option<CachedValue>> = values.into_iter().collect();
                self.merge_fetched(&returned);
            }
            Err(error) => tracing::debug!(
                error = %error,
                "a spend counter batch read fell back to a direct read"
            ),
        }
    }

    fn merge_fetched(&self, values: &BTreeMap<String, Option<CachedValue>>) {
        match decimal_map(values) {
            Ok(fetched) => {
                let mut state = lock(&self.state);
                for (key, value) in fetched {
                    state.loaded.entry(key).or_insert(value);
                }
            }
            Err(error) => tracing::debug!(
                error = %error,
                "a spend counter batch read fell back to a direct read"
            ),
        }
    }

    async fn wait_for_next(&self, generation: u64) {
        loop {
            let mut notified = std::pin::pin!(self.published.notified());
            notified.as_mut().enable();
            if self.completed.load(Ordering::SeqCst) != generation {
                return;
            }
            notified.await;
        }
    }
}

struct CounterRead<'batch> {
    batch: &'batch SpendCounterBatch,
    counter_key: &'batch str,
}

enum ReadStep {
    Wait(u64),
    Fetch,
}

impl CounterRead<'_> {
    // true when the snapshot can answer for the key now; false when this reader must go to Redis
    async fn settle(&self) -> bool {
        loop {
            let step = {
                let mut state = lock(&self.batch.state);
                if state.phase == BatchPhase::Closed || !state.keys.contains(self.counter_key) {
                    return false;
                }
                if state.fetching {
                    ReadStep::Wait(state.generation)
                } else if state.keys.is_subset(&state.fetched) && state.inflight.is_empty() {
                    return true;
                } else {
                    state.fetching = true;
                    ReadStep::Fetch
                }
            };
            match step {
                ReadStep::Wait(generation) => self.batch.wait_for_next(generation).await,
                ReadStep::Fetch => self.batch.fetch().await,
            }
        }
    }
}

// publishing on drop releases waiters even when the fetching reader is cancelled mid-flight
struct PublishOnDrop<'batch> {
    batch: &'batch SpendCounterBatch,
}

impl Drop for PublishOnDrop<'_> {
    fn drop(&mut self) {
        let generation = {
            let mut state = lock(&self.batch.state);
            state.fetching = false;
            state.generation = state.generation.saturating_add(1);
            state.generation
        };
        self.batch.completed.store(generation, Ordering::SeqCst);
        self.batch.published.notify_waiters();
    }
}
