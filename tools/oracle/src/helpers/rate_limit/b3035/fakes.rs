use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock, ScriptReply};
use betterlite_rate_limit::rate_limit_window_store::CounterScript;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use serde_json::{Value, json};

use super::{CacheValueInput, CounterValueInput, ReplyInput};

fn scalar_text(value: &RedisScalar) -> String {
    match value {
        RedisScalar::Text(text) => text.clone(),
        RedisScalar::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        RedisScalar::Float(number) => number.to_string(),
    }
}

fn planned_reply(plan: &[ReplyInput], index: usize) -> Option<ScriptReply> {
    match plan.get(index) {
        Some(ReplyInput::Values { values }) => Some(ScriptReply::Array(
            values
                .iter()
                .map(|value| match value {
                    CounterValueInput::Int(number) => ScriptReply::Int(*number),
                    CounterValueInput::Text(text) => ScriptReply::Text(text.clone()),
                })
                .collect(),
        )),
        Some(ReplyInput::Failure { .. }) | None => None,
    }
}

pub(super) struct PlannedScript {
    plan: Vec<ReplyInput>,
    calls: Mutex<Vec<Value>>,
}

impl PlannedScript {
    pub(super) const fn new(plan: Vec<ReplyInput>) -> Self {
        Self {
            plan,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, keys: &[String], args: &[RedisScalar]) -> usize {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(json!({
            "keys": keys,
            "args": args.iter().map(scalar_text).collect::<Vec<String>>(),
        }));
        calls.len().saturating_sub(1)
    }
}

#[async_trait::async_trait]
impl CounterScript for PlannedScript {
    fn source(&self) -> &'static str {
        "b3035 planned script"
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let index = self.record(keys, args);
        let Some(reply) = planned_reply(&self.plan, index) else {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "b3035 planned script",
            });
        };
        Ok(reply)
    }
}

struct RecordingState {
    values: BTreeMap<String, GaugeCacheValue>,
    reads: Vec<Value>,
    batch_reads: Vec<Value>,
    writes: Vec<Value>,
}

pub(super) struct RecordingCache {
    state: Mutex<RecordingState>,
}

fn gauge_value(value: &CacheValueInput) -> GaugeCacheValue {
    match value {
        CacheValueInput::Int(number) => GaugeCacheValue::Counter(CacheCounterValue::Int(*number)),
        CacheValueInput::Float(number) => {
            GaugeCacheValue::Counter(CacheCounterValue::Float(*number))
        }
        CacheValueInput::Text(text) => {
            GaugeCacheValue::Counter(CacheCounterValue::Text(text.clone()))
        }
        CacheValueInput::Registry(slots) => GaugeCacheValue::Registry(slots.clone()),
    }
}

fn recorded_value(value: &GaugeCacheValue) -> Value {
    match value {
        GaugeCacheValue::Registry(slots) => json!(slots),
        GaugeCacheValue::Counter(CacheCounterValue::Int(number)) => json!(number),
        GaugeCacheValue::Counter(CacheCounterValue::Float(number)) => json!(number),
        GaugeCacheValue::Counter(CacheCounterValue::Text(text)) => json!(text),
        GaugeCacheValue::Counter(CacheCounterValue::Bytes(bytes)) => json!(bytes),
    }
}

impl RecordingCache {
    pub(super) fn new(entries: &BTreeMap<String, CacheValueInput>) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| (key.clone(), gauge_value(value)))
            .collect();
        Self {
            state: Mutex::new(RecordingState {
                values,
                reads: Vec::new(),
                batch_reads: Vec::new(),
                writes: Vec::new(),
            }),
        }
    }

    pub(super) fn snapshot(&self) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (
            state.reads.clone(),
            state.batch_reads.clone(),
            state.writes.clone(),
        )
    }
}

#[async_trait::async_trait]
impl GaugeCache for RecordingCache {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .reads
            .push(json!({"key": key, "local_only": local_only}));
        state.values.get(key).cloned()
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.writes.push(json!({
            "key": key,
            "value": recorded_value(&value),
            "ttl": ttl.as_secs(),
            "local_only": local_only,
        }));
        state.values.insert(key.to_owned(), value);
    }

    async fn batch_get(
        &self,
        keys: &[String],
        local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .batch_reads
            .push(json!({"keys": keys, "local_only": local_only}));
        Some(
            keys.iter()
                .map(|key| state.values.get(key).cloned())
                .collect(),
        )
    }
}

pub(super) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(super) fn at(seconds: u64) -> Option<Self> {
        Some(Self {
            at: UNIX_EPOCH.checked_add(Duration::from_secs(seconds))?,
        })
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}
