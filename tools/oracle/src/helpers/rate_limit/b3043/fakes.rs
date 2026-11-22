use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock, ModelGroupResolver};
use betterlite_rate_limit::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind};
use betterlite_rate_limit::token_adjustments::store::UsageIncrements;
use betterlite_rate_limit::token_estimation::services::{
    GenAiTranslation, GenAiTranslationRequest, InputTokenCounter, ResponsesInput,
    ResponsesInputRequest, TokenCounterRequest, TranslatedChat,
};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde_json::{Value, json};

use crate::b3042::CounterValueInput;

#[derive(Default)]
struct Journal {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Vec<Value>,
    writes: Vec<Value>,
    increments: Vec<Value>,
    pipelines: Vec<Value>,
}

// one store behind the counter, gauge and usage calls, logged the way the Python fake logs them
pub(super) struct Recorder {
    journal: Mutex<Journal>,
}

fn counter_json(value: &CacheCounterValue) -> Value {
    match value {
        CacheCounterValue::Int(number) => json!(number),
        CacheCounterValue::Float(number) => json!(number),
        CacheCounterValue::Text(text) => json!(text),
        CacheCounterValue::Bytes(bytes) => json!(bytes),
    }
}

fn operation_json(operation: &IncrementOperation) -> Value {
    json!({
        "key": &operation.key,
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
    })
}

impl Recorder {
    pub(super) fn new(entries: &BTreeMap<String, CounterValueInput>) -> Self {
        let mut journal = Journal::default();
        for (key, entry) in entries {
            let value = match entry {
                CounterValueInput::Int(number) => CacheCounterValue::Int(*number),
                CounterValueInput::Float(number) => CacheCounterValue::Float(*number),
                CounterValueInput::Text(text) => CacheCounterValue::Text(text.clone()),
            };
            journal.values.insert(key.clone(), value);
        }
        Self {
            journal: Mutex::new(journal),
        }
    }

    pub(super) fn effects_json(&self) -> Value {
        let journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        json!({
            "reads": journal.reads,
            "writes": journal.writes,
            "increments": journal.increments,
            "pipelines": journal.pipelines,
        })
    }

    fn read(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        journal
            .reads
            .push(json!({"key": key, "local_only": local_only}));
        journal.values.get(key).cloned()
    }

    fn write(&self, key: &str, value: CacheCounterValue, ttl: Option<i64>, local_only: bool) {
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        let logged = counter_json(&value);
        journal.writes.push(json!({
            "key": key,
            "value": logged,
            "ttl": ttl,
            "local_only": local_only,
        }));
        journal.values.insert(key.to_owned(), value);
    }
}

#[async_trait::async_trait]
impl CounterCache for Recorder {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.read(key, local_only)
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        self.write(key, value, Some(ttl), local_only);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

#[async_trait::async_trait]
impl GaugeCache for Recorder {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue> {
        self.read(key, local_only).map(GaugeCacheValue::Counter)
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool) {
        // the cases seed gauges as plain counts, so a registry never comes back from a release
        if let GaugeCacheValue::Counter(counter) = value {
            let seconds = i64::try_from(ttl.as_secs()).ok();
            self.write(key, counter, seconds, local_only);
        }
    }

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

#[async_trait::async_trait]
impl UsageIncrements for Recorder {
    async fn increment(&self, operation: &IncrementOperation) {
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        journal.increments.push(operation_json(operation));
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        let logged: Vec<Value> = operations.iter().map(operation_json).collect();
        let mut journal = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        journal.pipelines.push(Value::Array(logged));
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        self.write(key, CacheCounterValue::Float(value), ttl, true);
    }
}

pub(super) struct Clock(pub(super) SystemTime);

impl LimiterClock for Clock {
    fn current_time(&self) -> SystemTime {
        self.0
    }
}

// the cases use inputs no tokenizer reads: pretokenized embeddings and an empty completion prompt
pub(super) struct Uncounted;

#[async_trait::async_trait]
impl InputTokenCounter for Uncounted {
    async fn count_tokens(&self, _request: &TokenCounterRequest<'_>) -> Option<i64> {
        None
    }
}

impl GenAiTranslation for Uncounted {
    fn translate_generate_content(
        &self,
        _request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        None
    }
}

impl ResponsesInput for Uncounted {
    fn transform_responses_api_input(&self, _request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        Vec::new()
    }
}

pub(super) struct NoGroups;

impl ModelGroupResolver for NoGroups {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

pub(super) struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}
