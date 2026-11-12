use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::batch::{MgetValues, RedisBatch};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use rust_decimal::Decimal;
use thiserror::Error;

use self::load::{LoadShared, Loader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterValue {
    // Redis held the counter with no value, so a direct read would find nothing either
    Missing,
    Amount(Decimal),
}

#[derive(Debug, Error)]
pub enum SpendCounterBatchError {
    #[error("the spend counter batch was cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSpendIncrement {
    pub counter_key: String,
    pub increment: Decimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchPhase {
    Open,
    Closed,
}

pub(super) struct BatchState {
    pub(super) phase: BatchPhase,
    pub(super) keys: BTreeSet<String>,
    pub(super) fetched: BTreeSet<String>,
    pub(super) loaded: BTreeMap<String, Option<Decimal>>,
    pub(super) inflight: Vec<BatchResult<MgetValues>>,
}

mod load;

pub struct SpendCounterBatch {
    redis_cache: Arc<RedisCache>,
    request_batch: Option<Arc<RedisBatch>>,
    pub(super) state: Mutex<BatchState>,
    loading: Mutex<Option<Arc<LoadShared>>>,
}

impl SpendCounterBatch {
    #[must_use]
    pub fn new(redis_cache: Arc<RedisCache>) -> Arc<Self> {
        let batch = Arc::new(Self {
            request_batch: active_request_redis_batch(&redis_cache),
            redis_cache,
            state: Mutex::new(BatchState {
                phase: BatchPhase::Open,
                keys: BTreeSet::new(),
                fetched: BTreeSet::new(),
                loaded: BTreeMap::new(),
                inflight: Vec::new(),
            }),
            loading: Mutex::new(None),
        });
        let Some(shared) = batch.request_batch.as_ref() else {
            return batch;
        };
        let declared = Arc::downgrade(&batch);
        shared.add_flush_hook(Arc::new(move || {
            let Some(held) = declared.upgrade() else {
                return;
            };
            held.declare_pending();
        }));
        batch
    }

    pub fn bind(&self, counter_keys: &BTreeSet<String>) {
        let mut state = lock(&self.state);
        if state.phase == BatchPhase::Open {
            state.keys.extend(counter_keys.iter().cloned());
        }
    }

    pub async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        let bound = {
            let state = lock(&self.state);
            state.phase == BatchPhase::Open && state.keys.contains(counter_key)
        };
        if !bound {
            return Ok(None);
        }
        let loaded = Loader { batch: self }.load().await?;
        let value = match loaded.get(counter_key) {
            Some(Some(amount)) => CounterValue::Amount(*amount),
            Some(None) => CounterValue::Missing,
            None => return Ok(None),
        };
        Ok(Some(value))
    }

    pub fn record(&self, counter_key: &str, value: Decimal) {
        let mut state = lock(&self.state);
        if state.phase == BatchPhase::Closed {
            return;
        }
        let key = counter_key.to_owned();
        state.keys.insert(key.clone());
        state.fetched.insert(key.clone());
        state.loaded.insert(key, Some(value));
    }

    pub fn forget(&self, counter_key: &str) {
        let mut state = lock(&self.state);
        state.keys.remove(counter_key);
        state.fetched.remove(counter_key);
        state.loaded.remove(counter_key);
    }

    pub(super) fn declare_pending(&self) {
        let Some(request_batch) = &self.request_batch else {
            return;
        };
        let mut state = lock(&self.state);
        if state.phase == BatchPhase::Closed {
            return;
        }
        let pending: Vec<String> = state.keys.difference(&state.fetched).cloned().collect();
        if pending.is_empty() {
            return;
        }
        state.fetched.extend(pending.iter().cloned());
        state.inflight.push(request_batch.mget(&pending));
    }
}
