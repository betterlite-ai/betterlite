use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use rust_decimal::Decimal;
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendCounterValue {
    // Redis held the counter with no value, so a direct read would find nothing either
    Missing,
    Amount(Decimal),
}

impl From<Option<Decimal>> for SpendCounterValue {
    fn from(value: Option<Decimal>) -> Self {
        match value {
            Some(amount) => Self::Amount(amount),
            None => Self::Missing,
        }
    }
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
    pub(super) generation: u64,
    pub(super) fetching: bool,
}

pub struct SpendCounterBatch {
    pub(super) redis_cache: Arc<RedisCache>,
    pub(super) request_batch: Option<Arc<RedisBatch>>,
    pub(super) state: Mutex<BatchState>,
    pub(super) completed: AtomicU64,
    pub(super) published: Notify,
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
                generation: 0,
                fetching: false,
            }),
            completed: AtomicU64::new(0),
            published: Notify::new(),
        });
        if let Some(request) = batch.request_batch.as_ref() {
            let declared = Arc::downgrade(&batch);
            request.add_flush_hook(Arc::new(move || {
                if let Some(held) = declared.upgrade() {
                    held.declare_pending();
                }
            }));
        }
        batch
    }

    pub fn bind(&self, counter_keys: &BTreeSet<String>) {
        let mut state = lock(&self.state);
        if state.phase == BatchPhase::Open {
            state.keys.extend(counter_keys.iter().cloned());
        }
    }

    pub fn record(&self, counter_key: &str, value: Decimal) {
        let mut state = lock(&self.state);
        match state.phase {
            BatchPhase::Closed => {}
            BatchPhase::Open => {
                state.keys.insert(counter_key.to_owned());
                state.fetched.insert(counter_key.to_owned());
                state.loaded.insert(counter_key.to_owned(), Some(value));
            }
        }
    }

    pub fn forget(&self, counter_key: &str) {
        let mut state = lock(&self.state);
        state.keys.remove(counter_key);
        state.fetched.remove(counter_key);
        state.loaded.remove(counter_key);
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        let phase = lock(&self.state).phase;
        match phase {
            BatchPhase::Open => true,
            BatchPhase::Closed => false,
        }
    }

    pub(super) fn declare_pending(&self) {
        let Some(request) = self.request_batch.as_ref() else {
            return;
        };
        let mut state = lock(&self.state);
        let pending: Vec<String> = match state.phase {
            BatchPhase::Closed => return,
            BatchPhase::Open => state.keys.difference(&state.fetched).cloned().collect(),
        };
        for key in &pending {
            state.fetched.insert(key.clone());
        }
        if !pending.is_empty() {
            state.inflight.push(request.mget(&pending));
        }
    }
}
