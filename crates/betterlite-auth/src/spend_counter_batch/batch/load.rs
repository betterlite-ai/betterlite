use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::Settled;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;
use tokio::sync::Notify;

use super::super::value::decimal_map;
use super::{SpendCounterBatch, SpendCounterBatchError};

pub(super) struct Loader<'batch> {
    pub(super) batch: &'batch SpendCounterBatch,
}

impl Loader<'_> {
    pub(super) async fn load(
        self,
    ) -> Result<BTreeMap<String, Option<Decimal>>, SpendCounterBatchError> {
        loop {
            let shared = match self.claim_load() {
                LoadClaim::Run(shared) => shared,
                LoadClaim::Wait(shared) => {
                    shared.wait().await;
                    continue;
                }
            };
            let finished = LoadFinished {
                batch: self.batch,
                shared,
            };
            if self.batch.request_batch.is_some() {
                self.batch.declare_pending();
                self.collect_inflight().await?;
            } else {
                self.fetch_pending().await;
            }
            drop(finished);
            return Ok(lock(&self.batch.state).loaded.clone());
        }
    }

    fn claim_load(&self) -> LoadClaim {
        let mut loading = lock(&self.batch.loading);
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
            let mut state = lock(&self.batch.state);
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
            let mut state = lock(&self.batch.state);
            let pending: BTreeSet<String> =
                state.keys.difference(&state.fetched).cloned().collect();
            state.fetched.extend(pending.iter().cloned());
            pending
        };
        if pending.is_empty() {
            return;
        }
        let requested: Vec<Option<&str>> = pending.iter().map(String::as_str).map(Some).collect();
        let fetched = match self
            .batch
            .redis_cache
            .async_batch_get_cache(&requested)
            .await
        {
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
        let mut state = lock(&self.batch.state);
        for (key, value) in fetched {
            state.loaded.entry(key).or_insert(value);
        }
    }
}

enum LoadClaim {
    Run(Arc<LoadShared>),
    Wait(Arc<LoadShared>),
}

pub(super) struct LoadShared {
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
