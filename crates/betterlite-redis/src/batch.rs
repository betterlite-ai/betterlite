mod delete;
pub mod error;
mod flush;
mod increment;
mod mget;
mod operation;
mod replies;
pub mod result;
mod script;
mod set;
mod slot;
mod tests;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use redis::Value;
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_util::task::TaskTracker;

use self::delete::DeleteOperation;
use self::increment::IncrementOperation;
use self::mget::MgetOperation;
use self::operation::Operation;
use self::result::{BatchResult, Settled};
use self::script::ScriptOperation;
use self::set::SetOperation;
use self::slot::Slot;
use crate::cache_primitives::decode::CachedValue;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_cache::RedisCache;
use crate::redis_counters::scripts::RegisteredScript;

pub type MgetValues = Arc<BTreeMap<String, Option<CachedValue>>>;

pub struct RedisBatch {
    redis_cache: Arc<RedisCache>,
    name: &'static str,
    flushes: Mutex<usize>,
    pending: Mutex<Vec<Operation>>,
    flush_hooks: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    misses: Arc<Mutex<BTreeSet<String>>>,
    follow_ups: TaskTracker,
    lock: AsyncMutex<()>,
}

impl RedisBatch {
    #[must_use]
    pub fn new(redis_cache: Arc<RedisCache>, name: &'static str, follow_ups: TaskTracker) -> Self {
        Self {
            redis_cache,
            name,
            flushes: Mutex::new(0),
            pending: Mutex::new(Vec::new()),
            flush_hooks: Mutex::new(Vec::new()),
            misses: Arc::new(Mutex::new(BTreeSet::new())),
            follow_ups,
            lock: AsyncMutex::new(()),
        }
    }

    pub fn mget(self: &Arc<Self>, keys: &[String]) -> BatchResult<MgetValues> {
        let (slot, receiver) = Slot::channel();
        let operation = MgetOperation::new(keys, &self.misses, &slot);
        self.declare(Operation::Mget(operation), slot, receiver)
    }

    pub fn script(
        self: &Arc<Self>,
        script: &Arc<RegisteredScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> BatchResult<Value> {
        let (slot, receiver) = Slot::channel();
        let operation = ScriptOperation {
            script: Arc::clone(script),
            keys: keys.to_vec(),
            args: args.to_vec(),
            slot: Arc::clone(&slot),
        };
        self.declare(Operation::Script(operation), slot, receiver)
    }

    pub fn increment(
        self: &Arc<Self>,
        key: &str,
        value: f64,
        ttl: Option<i64>,
    ) -> BatchResult<f64> {
        let (slot, receiver) = Slot::channel();
        let operation = IncrementOperation {
            key: key.to_owned(),
            value,
            ttl,
            slot: Arc::clone(&slot),
        };
        self.declare(Operation::Increment(operation), slot, receiver)
    }

    pub fn set<V>(self: &Arc<Self>, key: &str, value: &V, ttl: Option<f64>) -> BatchResult<()>
    where
        V: Serialize + ?Sized,
    {
        let (slot, receiver) = Slot::channel();
        let (text, encode_error) = match serde_json::to_string(value) {
            Ok(text) => (text, None),
            Err(error) => ("null".to_owned(), Some(error)),
        };
        match self.misses.lock() {
            Ok(mut misses) => {
                misses.remove(key);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(key);
            }
        }
        let operation = SetOperation {
            key: key.to_owned(),
            value: text,
            ttl,
            encode_error,
            slot: Arc::clone(&slot),
        };
        self.declare(Operation::Set(operation), slot, receiver)
    }

    pub fn delete(self: &Arc<Self>, key: &str) -> BatchResult<()> {
        let (slot, receiver) = Slot::channel();
        match self.misses.lock() {
            Ok(mut misses) => {
                misses.insert(key.to_owned());
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(key.to_owned());
            }
        }
        let operation = DeleteOperation {
            key: key.to_owned(),
            slot: Arc::clone(&slot),
        };
        self.declare(Operation::Delete(operation), slot, receiver)
    }

    pub fn add_flush_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        match self.flush_hooks.lock() {
            Ok(mut hooks) => hooks.push(hook),
            Err(poisoned) => poisoned.into_inner().push(hook),
        }
    }

    #[must_use]
    pub fn read_as_missing(&self, key: &str) -> bool {
        match self.misses.lock() {
            Ok(misses) => misses.contains(key),
            Err(poisoned) => poisoned.into_inner().contains(key),
        }
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        match self.pending.lock() {
            Ok(pending) => pending.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    #[must_use]
    pub fn flushes(&self) -> usize {
        match self.flushes.lock() {
            Ok(flushes) => *flushes,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub async fn flush(&self) {
        let _locked = self.lock.lock().await;
        flush::flush(self).await;
    }

    fn declare<T>(
        self: &Arc<Self>,
        operation: Operation,
        slot: Arc<Slot<T>>,
        receiver: oneshot::Receiver<Settled<T>>,
    ) -> BatchResult<T> {
        match self.pending.lock() {
            Ok(mut pending) => pending.push(operation),
            Err(poisoned) => poisoned.into_inner().push(operation),
        }
        BatchResult {
            batch: Arc::clone(self),
            slot,
            receiver,
        }
    }
}

fn deduplicated(keys: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::with_capacity(keys.len());
    for key in keys {
        if seen.insert(key.as_str()) {
            unique.push(key.clone());
        }
    }
    unique
}
