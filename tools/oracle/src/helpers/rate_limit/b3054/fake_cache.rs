use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_rate_limit::session_iteration_limits::cache::SessionIterationCache;

use super::input::RedisMode;

pub(super) struct FakeCache {
    mode: RedisMode,
    memory: Mutex<BTreeMap<String, i64>>,
    memory_ttls: Mutex<BTreeMap<String, u64>>,
    redis: Mutex<BTreeMap<String, i64>>,
    redis_ttls: Mutex<BTreeMap<String, u64>>,
}

pub(super) struct Snapshot {
    pub(super) memory: BTreeMap<String, i64>,
    pub(super) memory_ttls: BTreeMap<String, u64>,
    pub(super) redis: BTreeMap<String, i64>,
    pub(super) redis_ttls: BTreeMap<String, u64>,
}

impl FakeCache {
    pub(super) fn new(
        mode: RedisMode,
        memory: &BTreeMap<String, i64>,
        redis_values: &BTreeMap<String, i64>,
    ) -> Self {
        Self {
            mode,
            memory: Mutex::new(memory.clone()),
            memory_ttls: Mutex::new(BTreeMap::new()),
            redis: Mutex::new(redis_values.clone()),
            redis_ttls: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot {
            memory: self
                .memory
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            memory_ttls: self
                .memory_ttls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            redis: self
                .redis
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            redis_ttls: self
                .redis_ttls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    fn increment(&self, key: &str, ttl: Duration) -> i64 {
        let mut counts = self.redis.lock().unwrap_or_else(PoisonError::into_inner);
        let current = counts.get(key).copied().unwrap_or(0);
        let next = current.checked_add(1).unwrap_or(i64::MAX);
        counts.insert(key.to_owned(), next);
        drop(counts);
        if next == 1 {
            self.redis_ttls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned(), ttl.as_secs());
        }
        next
    }
}

impl SessionIterationCache for FakeCache {
    type Error = io::Error;
    type Script = ();

    fn redis_configured(&self) -> bool {
        self.mode != RedisMode::Absent
    }

    fn register_increment_script(&self, _script: &str) -> Option<()> {
        Some(())
    }

    fn run_increment_script(
        &self,
        _script: &(),
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<i64, io::Error>> + Send {
        if self.mode == RedisMode::ScriptFails {
            return ready(Err(io::Error::other("the fake Redis script failed")));
        }
        ready(Ok(self.increment(key, ttl)))
    }

    fn memory_count(&self, key: &str) -> impl Future<Output = Option<i64>> + Send {
        let guard = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        let stored = guard.get(key).copied();
        drop(guard);
        ready(stored)
    }

    fn memory_set_count(
        &self,
        key: &str,
        value: i64,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        self.memory_ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), ttl.as_secs());
        ready(())
    }
}
