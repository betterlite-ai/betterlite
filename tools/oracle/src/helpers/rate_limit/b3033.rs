use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_rate_limit::atomic_counter::in_memory::{
    LocalCounterCache,
    // ast-grep-ignore: no-long-names
    atomic_check_and_increment_in_memory as check_in_memory,
};
use betterlite_rate_limit::atomic_counter::payload::{
    IncrementAmounts,
    // ast-grep-ignore: no-long-names
    build_descriptor_atomic_payload as build_payload,
};
use betterlite_rate_limit::atomic_counter::response::build_atomic_response;
use betterlite_rate_limit::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, RateLimitDescriptor, RateLimitResponse,
    ReservationWindowIdentity,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CounterValueInput {
    Int(i64),
    Text(String),
}

impl CounterValueInput {
    fn counter_value(&self) -> CacheCounterValue {
        match self {
            Self::Int(number) => CacheCounterValue::Int(*number),
            Self::Text(text) => CacheCounterValue::Text(text.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    descriptor: RateLimitDescriptor,
    increment: IncrementAmounts,
    default_window_size: i64,
    now: i64,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
    reply: Option<Vec<CounterValueInput>>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct ReadRecord {
    key: String,
    local_only: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WriteRecord {
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

struct RecordingCache {
    state: Mutex<RecordingState>,
}

impl RecordingCache {
    fn new(entries: &BTreeMap<String, CounterValueInput>) -> Self {
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

    fn reads(&self) -> Vec<ReadRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
            .clone()
    }

    fn writes(&self) -> Vec<WriteRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .writes
            .clone()
    }
}

#[async_trait::async_trait]
impl LocalCounterCache for RecordingCache {
    async fn get(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.reads.push(ReadRecord {
            key: key.to_owned(),
            local_only,
        });
        state.values.get(key).cloned()
    }

    async fn set(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.writes.push(WriteRecord {
            key: key.to_owned(),
            value: counter_value_json(&value),
            ttl,
            local_only,
        });
        state.values.insert(key.to_owned(), value);
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

fn response_json(response: &RateLimitResponse) -> Value {
    let windows: Vec<&ReservationWindowIdentity> =
        response.reservation_windows.iter().flatten().collect();
    json!({
        "overall_code": response.overall_code.clone(),
        "statuses": response.statuses.clone(),
        "reservation_windows": windows,
    })
}

fn empty_answer() -> Value {
    json!({
        "response": response_json(&RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: Vec::new(),
            reservation_windows: None,
        }),
        "calls": [],
        "reads": [],
        "writes": [],
    })
}

fn lua_answer(
    reply: &[CounterValueInput],
    keys: &[String],
    args: &[i64],
    meta: &[AtomicCounterMeta],
) -> crate::helpers::HelperResult {
    let raw: Vec<CacheCounterValue> = reply.iter().map(CounterValueInput::counter_value).collect();
    let response = build_atomic_response(&raw, meta).or_value_error()?;
    Ok(json!({
        "response": response_json(&response),
        "calls": [{"keys": keys, "args": args}],
        "reads": [],
        "writes": [],
    }))
}

fn in_memory_answer(input: &Input, meta: &[AtomicCounterMeta]) -> crate::helpers::HelperResult {
    let cache = RecordingCache::new(&input.cache);
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let response = runtime
        .block_on(check_in_memory(&cache, meta, input.now))
        .or_value_error()?;
    Ok(json!({
        "response": response_json(&response),
        "calls": [],
        "reads": cache.reads(),
        "writes": cache.writes(),
    }))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let (keys, args, meta) = build_payload(
            &input.descriptor,
            input.increment,
            input.default_window_size,
        );
        if keys.is_empty() {
            return Ok(empty_answer());
        }
        input.reply.as_ref().map_or_else(
            || in_memory_answer(input, &meta),
            |reply| lua_answer(reply, &keys, &args, &meta),
        )
    }
}
