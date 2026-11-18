use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::Settled;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;
use tokio::pin;
use tokio::sync::Notify;

use super::super::value::decimal_map;
use super::{InflightRead, SpendCounterBatch, SpendCounterBatchError};

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
        let reads = {
            let mut state = lock(&self.batch.state);
            mem::take(&mut state.inflight)
        };
        for read in reads {
            let InflightRead { result, asked } = read;
            self.accept_settled(result.await, &asked)?;
        }
        Ok(())
    }

    fn accept_settled(
        &self,
        settled: Settled<MgetValues>,
        asked: &BTreeMap<String, Option<u64>>,
    ) -> Result<(), SpendCounterBatchError> {
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
            Ok(fetched) => self.merge(fetched, asked),
            Err(error) => tracing::debug!(
                error = %error,
                "spend counter batch read failed, falling back to per-key reads"
            ),
        }
        Ok(())
    }

    async fn fetch_pending(&self) {
        let asked = {
            let mut state = lock(&self.batch.state);
            state.take_pending()
        };
        if asked.is_empty() {
            return;
        }
        let requested: Vec<Option<&str>> = asked.keys().map(String::as_str).map(Some).collect();
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
        self.merge(fetched, &asked);
    }

    // a recorded write outranks whatever an older read reply brings back
    fn merge(
        &self,
        fetched: BTreeMap<String, Option<Decimal>>,
        asked: &BTreeMap<String, Option<u64>>,
    ) {
        let mut state = lock(&self.batch.state);
        for (key, value) in fetched {
            // a forget since the reply was asked for makes its value stale
            let current = state.generations.get(&key).copied();
            if asked.get(&key) != Some(&current) {
                continue;
            }
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
            pin!(notified);
            notified.as_mut().enable();
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
