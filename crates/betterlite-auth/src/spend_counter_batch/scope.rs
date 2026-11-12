use std::collections::BTreeSet;
use std::mem;
use std::sync::{Arc, Mutex};

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::spend_counter_batch::batch::{
    BatchPhase, CounterValue, SpendCounterBatch, SpendCounterBatchError,
};
use crate::spend_counter_batch::keys::admission_counter_keys;

#[derive(Default)]
pub struct SpendCounterScope {
    pub(crate) active: Mutex<Option<Arc<SpendCounterBatch>>>,
}

#[must_use = "the active batch is restored when the guard drops"]
pub struct SpendCounterScopeGuard<'scope> {
    scope: &'scope SpendCounterScope,
    restore: ScopeRestore,
}

enum ScopeRestore {
    Unchanged,
    Previous(Option<Arc<SpendCounterBatch>>),
}

impl SpendCounterScope {
    pub fn enter(
        &self,
        redis_cache: Option<&Arc<RedisCache>>,
        counter_keys: &BTreeSet<String>,
    ) -> SpendCounterScopeGuard<'_> {
        let Some(cache) = redis_cache else {
            return SpendCounterScopeGuard {
                scope: self,
                restore: ScopeRestore::Unchanged,
            };
        };
        let mut active = lock(&self.active);
        let reusable = active
            .as_ref()
            .is_some_and(|outer| lock(&outer.state).phase == BatchPhase::Open);
        if reusable {
            if let Some(outer) = active.as_ref() {
                outer.bind(counter_keys);
            }
            return SpendCounterScopeGuard {
                scope: self,
                restore: ScopeRestore::Unchanged,
            };
        }
        let batch = SpendCounterBatch::new(Arc::clone(cache));
        batch.bind(counter_keys);
        let previous = active.replace(batch);
        drop(active);
        SpendCounterScopeGuard {
            scope: self,
            restore: ScopeRestore::Previous(previous),
        }
    }

    pub fn bind_spend_counter_keys(&self, counter_keys: &BTreeSet<String>) {
        if let Some(batch) = lock(&self.active).as_ref() {
            batch.bind(counter_keys);
        }
    }

    pub fn record_spend_counter_value(&self, counter_key: &str, value: Decimal) {
        if let Some(batch) = lock(&self.active).as_ref() {
            batch.record(counter_key, value);
        }
    }

    pub fn forget_spend_counter(&self, counter_key: &str) {
        if let Some(batch) = lock(&self.active).as_ref() {
            batch.forget(counter_key);
        }
    }

    pub async fn read_spend_counter(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        let active = lock(&self.active).clone();
        match active {
            Some(batch) => batch.read(counter_key).await,
            None => Ok(None),
        }
    }

    pub async fn read_batched_spend_counter(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        let window = BTreeSet::from([counter_key.to_owned()]);
        self.bind_spend_counter_keys(&window);
        self.read_spend_counter(counter_key).await
    }

    pub fn bind_admission_counter_keys(&self, auth: &UserAPIKeyAuth, end_user_id: Option<&str>) {
        if let Some(batch) = lock(&self.active).as_ref() {
            batch.bind(&admission_counter_keys(auth, end_user_id));
        }
    }

    pub fn release_spend_counter_batch(&self) {
        if let Some(batch) = lock(&self.active).as_ref() {
            lock(&batch.state).phase = BatchPhase::Closed;
        }
    }
}

impl Drop for SpendCounterScopeGuard<'_> {
    fn drop(&mut self) {
        let restore = mem::replace(&mut self.restore, ScopeRestore::Unchanged);
        let ScopeRestore::Previous(previous) = restore else {
            return;
        };
        *lock(&self.scope.active) = previous;
    }
}
