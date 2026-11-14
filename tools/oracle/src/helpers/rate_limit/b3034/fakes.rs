use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock, ScriptReply};
use betterlite_rate_limit::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, RequestBatchSource, ScriptBatch,
    ScriptHandle,
};
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use serde_json::{Value, json};

use super::{CounterValueInput, ReplyInput};

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct CallRecord {
    pub(super) keys: Vec<String>,
    pub(super) args: Vec<String>,
}

impl CallRecord {
    fn new(keys: &[String], args: &[RedisScalar]) -> Self {
        Self {
            keys: keys.to_vec(),
            args: args.iter().map(scalar_text).collect(),
        }
    }
}

fn scalar_text(value: &RedisScalar) -> String {
    match value {
        RedisScalar::Text(text) => text.clone(),
        RedisScalar::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        RedisScalar::Float(number) => number.to_string(),
    }
}

fn reply_from_values(values: &[CounterValueInput]) -> ScriptReply {
    ScriptReply::Array(
        values
            .iter()
            .map(|value| match value {
                CounterValueInput::Int(number) => ScriptReply::Int(*number),
                CounterValueInput::Text(text) => ScriptReply::Text(text.clone()),
            })
            .collect(),
    )
}

fn planned_reply(plan: &[ReplyInput], index: usize) -> Option<ScriptReply> {
    match plan.get(index) {
        Some(ReplyInput::Values { values }) => Some(reply_from_values(values)),
        Some(ReplyInput::Failure { .. }) | None => None,
    }
}

pub(super) struct PlannedScript {
    plan: Vec<ReplyInput>,
    calls: Mutex<Vec<CallRecord>>,
}

impl PlannedScript {
    pub(super) const fn new(plan: Vec<ReplyInput>) -> Self {
        Self {
            plan,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<CallRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, keys: &[String], args: &[RedisScalar]) -> usize {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(CallRecord::new(keys, args));
        calls.len().saturating_sub(1)
    }
}

#[async_trait::async_trait]
impl CounterScript for PlannedScript {
    fn source(&self) -> &str {
        "b3034 planned script"
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let index = self.record(keys, args);
        let Some(reply) = planned_reply(&self.plan, index) else {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "b3034 planned script",
            });
        };
        Ok(reply)
    }
}

pub(super) struct PlannedBatch {
    plan: Vec<ReplyInput>,
    calls: Mutex<Vec<CallRecord>>,
}

impl PlannedBatch {
    pub(super) const fn new(plan: Vec<ReplyInput>) -> Self {
        Self {
            plan,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<CallRecord> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, keys: &[String], args: &[RedisScalar]) -> usize {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(CallRecord::new(keys, args));
        calls.len().saturating_sub(1)
    }
}

impl ScriptBatch for PlannedBatch {
    fn script(
        &self,
        _script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> ScriptHandle {
        let index = self.record(keys, args);
        let reply = match planned_reply(&self.plan, index) {
            Some(reply) => Ok(reply),
            None => Err(Arc::new(BatchError::Cancelled)),
        };
        Box::pin(async move { reply })
    }
}

pub(super) struct PlannedBatchSource {
    pub(super) batch: Arc<PlannedBatch>,
}

impl RequestBatchSource for PlannedBatchSource {
    fn active_batch(&self) -> Option<Arc<dyn ScriptBatch>> {
        let shared: Arc<dyn ScriptBatch> = Arc::<PlannedBatch>::clone(&self.batch);
        Some(shared)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct ReadRecord {
    key: String,
    local_only: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct WriteRecord {
    key: String,
    value: Value,
    ttl: i64,
    local_only: bool,
}

#[derive(Default)]
struct RecordingState {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Vec<ReadRecord>,
    writes: Vec<WriteRecord>,
}

pub(super) struct RecordingCache {
    state: Mutex<RecordingState>,
}

impl RecordingCache {
    pub(super) fn new(entries: &BTreeMap<String, CounterValueInput>) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| (key.clone(), value.counter_value()))
            .collect();
        Self {
            state: Mutex::new(RecordingState {
                values,
                reads: Vec::new(),
                writes: Vec::new(),
            }),
        }
    }

    pub(super) fn reads(&self) -> Vec<ReadRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
            .clone()
    }

    pub(super) fn writes(&self) -> Vec<WriteRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .writes
            .clone()
    }
}

fn counter_value_json(value: &CacheCounterValue) -> Value {
    match value {
        CacheCounterValue::Int(number) => json!(number),
        CacheCounterValue::Float(number) => json!(number),
        CacheCounterValue::Text(text) => json!(text),
        CacheCounterValue::Bytes(bytes) => json!(bytes),
    }
}

#[async_trait::async_trait]
impl CounterCache for RecordingCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.reads.push(ReadRecord {
            key: key.to_owned(),
            local_only,
        });
        state.values.get(key).cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.writes.push(WriteRecord {
            key: key.to_owned(),
            value: counter_value_json(&value),
            ttl,
            local_only,
        });
        state.values.insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(super) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(super) fn at(seconds: u64) -> Self {
        Self {
            at: UNIX_EPOCH + Duration::from_secs(seconds),
        }
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}
