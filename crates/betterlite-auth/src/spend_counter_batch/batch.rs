use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{BatchResult, Settled};
use betterlite_redis::batch::{MgetValues, RedisBatch};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Notify;

use super::value::decimal_map;

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

pub struct SpendCounterBatch {
    redis_cache: Arc<RedisCache>,
    request_batch: Option<Arc<RedisBatch>>,
    pub(super) state: Mutex<BatchState>,
    loading: Mutex<Option<Arc<LoadShared>>>,
}

impl SpendCounterBatch {
    #[must_use]
    pub fn new(redis_cache: Arc<RedisCache>) -> Arc<Self> {
        let request_batch = active_request_redis_batch(&redis_cache);
        let batch = Arc::new(Self {
            request_batch: request_batch.clone(),
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
        let Some(shared) = request_batch else {
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

    pub fn close(&self) {
        let mut state = lock(&self.state);
        if state.phase == BatchPhase::Open {
            state.phase = BatchPhase::Closed;
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
        let loaded = self.load().await?;
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

    async fn load(&self) -> Result<BTreeMap<String, Option<Decimal>>, SpendCounterBatchError> {
        loop {
            let shared = match self.claim_load() {
                LoadClaim::Run(shared) => shared,
                LoadClaim::Wait(shared) => {
                    shared.wait().await;
                    continue;
                }
            };
            let finished = LoadFinished {
                batch: self,
                shared,
            };
            if self.request_batch.is_some() {
                self.declare_pending();
                self.collect_inflight().await?;
            } else {
                self.fetch_pending().await;
            }
            drop(finished);
            return Ok(lock(&self.state).loaded.clone());
        }
    }

    fn claim_load(&self) -> LoadClaim {
        let mut loading = lock(&self.loading);
        if let Some(shared) = loading.as_ref() {
            return LoadClaim::Wait(Arc::clone(shared));
        }
        let shared = Arc::new(LoadShared {
            complete: AtomicBool::new(false),
            done: Notify::new(),
        });
        *loading = Some(Arc::clone(&shared));
        drop(loading);
        LoadClaim::Run(shared)
    }

    async fn collect_inflight(&self) -> Result<(), SpendCounterBatchError> {
        let results = {
            let mut state = lock(&self.state);
            mem::take(&mut state.inflight)
        };
        for result in results {
            self.accept_settled(result.await)?;
        }
        Ok(())
    }

    fn accept_settled(&self, settled: Settled<MgetValues>) -> Result<(), SpendCounterBatchError> {
        let values = match settled {
            Ok(values) => values,
            Err(error) if matches!(error.as_ref(), BatchError::Cancelled) => {
                return Err(SpendCounterBatchError::Cancelled);
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "spend counter batch read failed, falling back to per-key reads"
                );
                return Ok(());
            }
        };
        match decimal_map(values.iter()) {
            Ok(fetched) => self.merge(fetched),
            Err(error) => tracing::debug!(
                error = %error,
                "spend counter batch read failed, falling back to per-key reads"
            ),
        }
        Ok(())
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
        let requested: Vec<Option<&str>> = pending.iter().map(String::as_str).map(Some).collect();
        let fetched = match self.redis_cache.async_batch_get_cache(&requested).await {
            Ok(values) => match decimal_map(values.iter()) {
                Ok(fetched) => fetched,
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "spend counter batch read failed, falling back to per-key reads"
                    );
                    BTreeMap::new()
                }
            },
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "spend counter batch read failed, falling back to per-key reads"
                );
                BTreeMap::new()
            }
        };
        self.merge(fetched);
    }

    // a recorded write outranks whatever an older read reply brings back
    fn merge(&self, fetched: BTreeMap<String, Option<Decimal>>) {
        let mut state = lock(&self.state);
        for (key, value) in fetched {
            state.loaded.entry(key).or_insert(value);
        }
    }
}

enum LoadClaim {
    Run(Arc<LoadShared>),
    Wait(Arc<LoadShared>),
}

struct LoadShared {
    complete: AtomicBool,
    done: Notify,
}

impl LoadShared {
    fn complete(&self) {
        self.complete.store(true, Ordering::SeqCst);
        self.done.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.done.notified();
            if self.complete.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

// completing on drop lets a cancelled reader release waiters instead of stranding them
struct LoadFinished<'batch> {
    batch: &'batch SpendCounterBatch,
    shared: Arc<LoadShared>,
}

impl Drop for LoadFinished<'_> {
    fn drop(&mut self) {
        {
            let mut loading = lock(&self.batch.loading);
            if loading
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.shared))
            {
                *loading = None;
            }
        }
        self.shared.complete();
    }
}
