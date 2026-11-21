use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_rate_limit::limiter_records::{CacheCounterValue, ScriptReply};
use betterlite_rate_limit::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind,
};
use betterlite_rate_limit::session_iteration_limits::INCREMENT_ITERATIONS_SCRIPT;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;

use super::input::RedisMode;

#[derive(Clone)]
pub(super) struct FakeCache {
    mode: RedisMode,
    state: Arc<Mutex<Snapshot>>,
}

struct FakeScript {
    mode: RedisMode,
    state: Arc<Mutex<Snapshot>>,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) memory: BTreeMap<String, i64>,
    pub(super) memory_ttls: BTreeMap<String, i64>,
    pub(super) redis: BTreeMap<String, i64>,
    pub(super) redis_ttls: BTreeMap<String, i64>,
}

impl FakeCache {
    pub(super) fn new(
        mode: RedisMode,
        memory: &BTreeMap<String, i64>,
        redis_values: &BTreeMap<String, i64>,
    ) -> Self {
        Self {
            mode,
            state: Arc::new(Mutex::new(Snapshot {
                memory: memory.clone(),
                memory_ttls: BTreeMap::new(),
                redis: redis_values.clone(),
                redis_ttls: BTreeMap::new(),
            })),
        }
    }

    pub(super) fn script(&self) -> Arc<dyn CounterScript> {
        Arc::new(FakeScript {
            mode: self.mode,
            state: Arc::clone(&self.state),
        })
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl CounterCache for FakeCache {
    async fn get_cache(&self, key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.memory.get(key).copied().map(CacheCounterValue::Int)
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, _local_only: bool) {
        let count = match value {
            CacheCounterValue::Int(count) => count,
            CacheCounterValue::Float(_)
            | CacheCounterValue::Text(_)
            | CacheCounterValue::Bytes(_) => return,
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.memory.insert(key.to_owned(), count);
        state.memory_ttls.insert(key.to_owned(), ttl);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        match self.mode {
            RedisMode::Absent => None,
            RedisMode::Healthy | RedisMode::ScriptFails => Some(RedisCacheKind::Standalone),
        }
    }
}

#[async_trait::async_trait]
impl CounterScript for FakeScript {
    fn source(&self) -> &'static str {
        INCREMENT_ITERATIONS_SCRIPT
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let unexpected = RedisCountersError::UnexpectedReply {
            operation: "the fake increment script",
        };
        let (RedisMode::Healthy, [key], [RedisScalar::Text(ttl_text)]) = (self.mode, keys, args)
        else {
            return Err(unexpected);
        };
        let Ok(ttl) = ttl_text.parse::<i64>() else {
            return Err(unexpected);
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let next = state.redis.get(key).copied().unwrap_or(0).saturating_add(1);
        state.redis.insert(key.clone(), next);
        if next == 1 {
            state.redis_ttls.insert(key.clone(), ttl);
        }
        drop(state);
        Ok(ScriptReply::Int(next))
    }
}
