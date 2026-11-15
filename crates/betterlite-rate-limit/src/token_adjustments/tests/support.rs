use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::batch::result::{Settled, SettledHook};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex as AsyncMutex;

use crate::limiter_records::{
    CacheCounterValue, ReservationAwareIncrement, ReservationBackend, ScriptReply,
};
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, WindowStore,
};
use crate::token_adjustments::store::{PostCallScriptBatch, TokenAdjustments, UsageIncrements};

pub(crate) const WINDOW_START: &str = "100";

// f64 holds whole counts exactly to 2^53, and the next one rounds
pub(crate) const MAX_EXACT_COUNT: i64 = 0x0020_0000_0000_0000;
pub(crate) const FIRST_INEXACT_COUNT: i64 = 0x0020_0000_0000_0001;

pub(crate) fn operation(key: &str, increment: f64, ttl: Option<i64>) -> IncrementOperation {
    IncrementOperation {
        key: key.to_owned(),
        increment_value: increment,
        ttl,
    }
}

pub(crate) fn operation_tuples(
    operations: &[IncrementOperation],
) -> Vec<(String, f64, Option<i64>)> {
    operations
        .iter()
        .map(|operation| {
            (
                operation.key.clone(),
                operation.increment_value,
                operation.ttl,
            )
        })
        .collect()
}

pub(crate) fn pipeline_tuples(
    pipelines: &[Vec<IncrementOperation>],
) -> Vec<Vec<(String, f64, Option<i64>)>> {
    let mut tuples = Vec::new();
    for operations in pipelines {
        tuples.push(operation_tuples(operations));
    }
    tuples
}

pub(crate) fn guarded_operation(
    key: &str,
    increment: f64,
    backend: ReservationBackend,
) -> ReservationAwareIncrement {
    let mut reservation = unguarded_operation(key, increment);
    reservation.window_key = Some("{scope:value}:window".to_owned());
    reservation.expected_window_start = Some(WINDOW_START.to_owned());
    reservation.reservation_backend = Some(backend);
    reservation
}

pub(crate) fn unguarded_operation(key: &str, increment: f64) -> ReservationAwareIncrement {
    ReservationAwareIncrement {
        operation: operation(key, increment, Some(60)),
        window_key: None,
        expected_window_start: None,
        reservation_backend: None,
    }
}

pub(crate) struct FakeCache {
    kind: Option<RedisCacheKind>,
    stored: Mutex<BTreeMap<String, CacheCounterValue>>,
    reads: Mutex<Vec<(String, bool)>>,
}

impl FakeCache {
    pub(crate) fn new(kind: Option<RedisCacheKind>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            stored: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn seed(&self, key: &str, value: CacheCounterValue) {
        self.stored
            .lock()
            .expect("the cache lock")
            .insert(key.to_owned(), value);
    }

    pub(crate) fn reads(&self) -> Vec<(String, bool)> {
        self.reads.lock().expect("the cache lock").clone()
    }
}

#[async_trait::async_trait]
impl CounterCache for FakeCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), local_only));
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, _ttl: i64, _local_only: bool) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, keys: &[String], _local_only: bool) -> Option<CounterValues> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        self.kind
    }
}

#[derive(Default)]
pub(crate) struct FakeUsage {
    increments: Mutex<Vec<IncrementOperation>>,
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
    local_sets: Mutex<Vec<(String, f64, Option<i64>)>>,
}

impl FakeUsage {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn increments(&self) -> Vec<IncrementOperation> {
        self.increments.lock().expect("the usage lock").clone()
    }

    pub(crate) fn pipelines(&self) -> Vec<Vec<IncrementOperation>> {
        self.pipelines.lock().expect("the usage lock").clone()
    }

    pub(crate) fn local_sets(&self) -> Vec<(String, f64, Option<i64>)> {
        self.local_sets.lock().expect("the usage lock").clone()
    }
}

#[async_trait::async_trait]
impl UsageIncrements for FakeUsage {
    async fn increment(&self, operation: &IncrementOperation) {
        self.increments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operation.clone());
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operations.to_vec());
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        self.local_sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
    }
}

pub(crate) struct FakeScript {
    source: &'static str,
    reply: ScriptReply,
    fail_from: AtomicUsize,
    calls: Mutex<Vec<(Vec<String>, Vec<RedisScalar>)>>,
}

impl FakeScript {
    pub(crate) fn new(source: &'static str) -> Arc<Self> {
        Arc::new(Self {
            source,
            reply: ScriptReply::Array(Vec::new()),
            fail_from: AtomicUsize::new(usize::MAX),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn fail(&self) {
        self.fail_from.store(1, Ordering::SeqCst);
    }

    pub(crate) fn fail_from(&self, call: usize) {
        self.fail_from.store(call, Ordering::SeqCst);
    }

    pub(crate) fn calls(&self) -> Vec<(Vec<String>, Vec<RedisScalar>)> {
        self.calls.lock().expect("the script lock").clone()
    }
}

#[async_trait::async_trait]
impl CounterScript for FakeScript {
    fn source(&self) -> &str {
        self.source
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push((keys.to_vec(), args.to_vec()));
        let call = calls.len();
        drop(calls);
        if call >= self.fail_from.load(Ordering::SeqCst) {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "token increment script",
            });
        }
        Ok(self.reply.clone())
    }
}

pub(crate) type BatchCall = (String, Vec<String>, Vec<RedisScalar>);

pub(crate) struct FakeBatch {
    calls: Mutex<Vec<BatchCall>>,
    hooks: Mutex<Vec<SettledHook<ScriptReply>>>,
}

impl FakeBatch {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            hooks: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<BatchCall> {
        self.calls.lock().expect("the batch lock").clone()
    }

    pub(crate) async fn settle(&self, result: &Settled<ScriptReply>) {
        let hooks: Vec<SettledHook<ScriptReply>> =
            self.hooks.lock().expect("the batch lock").clone();
        for hook in hooks {
            if let Some(follow_up) = hook(result) {
                follow_up.await;
            }
        }
    }
}

impl PostCallScriptBatch for FakeBatch {
    fn script_on_settled(
        &self,
        script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
        hook: SettledHook<ScriptReply>,
    ) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((script.source().to_owned(), keys.to_vec(), args.to_vec()));
        self.hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }
}

pub(crate) fn adjustments_with(
    cache: Arc<FakeCache>,
    usage: Arc<FakeUsage>,
    token_script: Option<Arc<dyn CounterScript>>,
    guarded_script: Option<Arc<dyn CounterScript>>,
    batch: Option<Arc<dyn PostCallScriptBatch>>,
) -> TokenAdjustments {
    TokenAdjustments {
        windows: WindowStore {
            cache,
            rate_limiter_script: None,
            counter_read_script: None,
            request_batch: None,
            check_and_increment_lock: AsyncMutex::new(()),
        },
        usage,
        post_call_batch: batch,
        token_increment_script: token_script,
        guarded_token_script: guarded_script,
    }
}

pub(crate) fn without_scripts(cache: &Arc<FakeCache>, usage: &Arc<FakeUsage>) -> TokenAdjustments {
    adjustments_with(Arc::clone(cache), Arc::clone(usage), None, None, None)
}

pub(crate) fn batch_handle(batch: &Arc<FakeBatch>) -> Arc<dyn PostCallScriptBatch> {
    Arc::<FakeBatch>::clone(batch)
}

pub(crate) fn script_handle<S>(script: &Arc<S>) -> Arc<dyn CounterScript>
where
    S: CounterScript + 'static,
{
    Arc::<S>::clone(script)
}
