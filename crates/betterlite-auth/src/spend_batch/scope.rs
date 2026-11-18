use std::collections::BTreeSet;
use std::mem;
use std::sync::{Arc, Mutex};

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use super::batch::{BatchPhase, SpendCounterBatch, SpendCounterValue};
use super::keys::admission_counter_keys;
use crate::resolved_key::auth::UserAPIKeyAuth;

#[derive(Default)]
pub struct SpendBatchContext {
    active: Mutex<Option<Arc<SpendCounterBatch>>>,
}

#[must_use = "the previous active batch is restored when the scope exits"]
pub struct SpendBatchScope<'context> {
    context: &'context SpendBatchContext,
    restore: ScopeRestore,
}

enum ScopeRestore {
    Unchanged,
    Previous(Option<Arc<SpendCounterBatch>>),
}

impl SpendBatchContext {
    pub fn enter(
        &self,
        redis_cache: Option<&Arc<RedisCache>>,
        counter_keys: &BTreeSet<String>,
    ) -> SpendBatchScope<'_> {
        let Some(redis_cache) = redis_cache else {
            return SpendBatchScope {
                context: self,
                restore: ScopeRestore::Unchanged,
            };
        };
        let mut active = lock(&self.active);
        if let Some(outer) = active.as_ref()
            && outer.is_open()
        {
            outer.bind(counter_keys);
            return SpendBatchScope {
                context: self,
                restore: ScopeRestore::Unchanged,
            };
        }
        let batch = SpendCounterBatch::new(Arc::clone(redis_cache));
        batch.bind(counter_keys);
        let previous = active.replace(batch);
        drop(active);
        SpendBatchScope {
            context: self,
            restore: ScopeRestore::Previous(previous),
        }
    }

    #[must_use]
    pub fn active_spend_counter_batch(&self) -> Option<Arc<SpendCounterBatch>> {
        let active = lock(&self.active);
        let batch = active.as_ref()?;
        Some(Arc::clone(batch))
    }

    pub fn bind_spend_counter_keys(&self, counter_keys: &BTreeSet<String>) {
        if let Some(batch) = self.active_spend_counter_batch() {
            batch.bind(counter_keys);
        }
    }

    pub fn bind_admission_counter_keys(&self, auth: &UserAPIKeyAuth, end_user_id: Option<&str>) {
        if let Some(batch) = self.active_spend_counter_batch() {
            batch.bind(&admission_counter_keys(auth, end_user_id));
        }
    }

    pub fn record_spend_counter_value(&self, counter_key: &str, value: Decimal) {
        if let Some(batch) = self.active_spend_counter_batch() {
            batch.record(counter_key, value);
        }
    }

    pub fn forget_spend_counter(&self, counter_key: &str) {
        if let Some(batch) = self.active_spend_counter_batch() {
            batch.forget(counter_key);
        }
    }

    pub fn release_spend_counter_batch(&self) {
        let Some(batch) = self.active_spend_counter_batch() else {
            return;
        };
        lock(&batch.state).phase = BatchPhase::Closed;
    }

    pub async fn read_spend_counter(&self, counter_key: &str) -> Option<SpendCounterValue> {
        match self.active_spend_counter_batch() {
            Some(batch) => batch.read(counter_key).await,
            None => None,
        }
    }

    pub async fn read_batched_spend_counter(&self, counter_key: &str) -> Option<SpendCounterValue> {
        match self.active_spend_counter_batch() {
            Some(batch) => {
                batch.bind(&BTreeSet::from([counter_key.to_owned()]));
                batch.read(counter_key).await
            }
            None => None,
        }
    }
}

impl Drop for SpendBatchScope<'_> {
    fn drop(&mut self) {
        let restore = mem::replace(&mut self.restore, ScopeRestore::Unchanged);
        let ScopeRestore::Previous(previous) = restore else {
            return;
        };
        *lock(&self.context.active) = previous;
    }
}
