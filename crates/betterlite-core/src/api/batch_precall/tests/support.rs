use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::rate_limit::{ProxyRateLimitError, ProxyRateLimitErrorInit};
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_enqueued_refund::EnqueuedTokenRefund;
use crate::api::batch_enqueued_refund::store::{EnqueuedLocalStore as _, ReservationRecordScripts};
use crate::api::batch_enqueued_reserve::store::{
    EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts, ReserveScriptReply,
};
use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, BatchEnqueuedTokenSettings,
    ReservationBackend, ScopeKey,
};
use crate::api::batch_input_accounting::error::BatchHttpError;
use crate::api::batch_limit_counters::records::{
    BatchFileUsage, BatchReservationBackend, BatchTpdRefundOperation, EpochClock,
};
use crate::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::batch_precall::input::{
    BatchCacheControl, BatchCreateCall, BatchPrecallCall, BatchPrecallInput, BatchRetrieveCall,
    BatchTerminalCall,
};
use crate::api::batch_precall::ports::{
    BatchAccountRequest, BatchCacheError, BatchPrecallMemory, BatchPrecallRemote, BatchPrecallSkip,
    BatchPrecallStages, BatchPrecallStash, BatchTokenApplier,
};
use crate::api::batch_precall::result::{
    BatchDiagnostic, BatchDiagnosticLevel, BatchPrecallResult,
};
use crate::api::batch_precall::{BatchPrecall, BatchPrecallError};
use crate::api::request_destination::RequestData;

pub(crate) const KEY: &str = "sk-key";
pub(crate) const CALL_TYPE: &str = "acreate_batch";
pub(crate) const RETRIEVE_CALL_TYPE: &str = "aretrieve_batch";
pub(crate) const CACHE_NAME: &str = "litellm:sk-key:aretrieve_batch";
pub(crate) const CACHE_KEY: &str = "litellm:sk-key:aretrieve_batch:abc";
pub(crate) const COUNTER_KEY: &str = "batch_enqueued_tokens:api_key:sk-key";
pub(crate) const OPERATOR: &str = "worker-1";

fn text_data(fields: &[(&str, &str)]) -> RequestData {
    let mut data = RequestData::new();
    for (name, value) in fields {
        data.insert(String::from(*name), Json::Text(String::from(*value)));
    }
    data.insert(String::from("metadata"), Json::Object(IndexMap::new()));
    data
}

pub(crate) fn create_data(file_id: &str) -> RequestData {
    text_data(&[("model", "gpt-4o-mini"), ("input_file_id", file_id)])
}

pub(crate) fn retrieve_data() -> RequestData {
    text_data(&[("batch_id", "batch-1")])
}

pub(crate) fn principal_with_scope() -> BatchLimitPrincipal {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        String::from("batch_enqueued_token_limit"),
        JsonData::Number(Number::from(1_000_i64)),
    );
    let mut fields = BTreeMap::new();
    fields.insert(String::from("api_key"), JsonData::String(KEY.to_owned()));
    fields.insert(String::from("metadata"), JsonData::Object(metadata));
    BatchLimitPrincipal(fields)
}

pub(crate) fn plain_principal() -> BatchLimitPrincipal {
    let mut fields = BTreeMap::new();
    fields.insert(String::from("api_key"), JsonData::String(KEY.to_owned()));
    BatchLimitPrincipal(fields)
}

pub(crate) fn key_scope() -> BatchEnqueuedTokenScope {
    BatchEnqueuedTokenScope {
        key: ScopeKey::ApiKey,
        value: KEY.to_owned(),
        limit: 1_000,
    }
}

pub(crate) fn file_usage(tokens: i64, requests: i64) -> BatchFileUsage {
    BatchFileUsage {
        total_tokens: tokens,
        request_count: requests,
        output_tokens: 0,
        per_model_usage: IndexMap::new(),
    }
}

pub(crate) fn descriptor() -> BatchLimitDescriptor {
    BatchLimitDescriptor {
        key: String::from("api_key"),
        value: KEY.to_owned(),
        rate_limit: None,
    }
}

pub(crate) fn rejection() -> Box<ProxyRateLimitError> {
    let init = ProxyRateLimitErrorInit {
        detail: Payload::String(String::from("batch rate limit reached")),
        headers: None,
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    };
    Box::new(ProxyRateLimitError::new(init).expect("the rejection carries no headers"))
}

pub(crate) fn operation() -> BatchTpdRefundOperation {
    BatchTpdRefundOperation {
        key: String::from("api_key_tpd"),
        increment_value: 120,
        ttl: 86_400,
        window_key: String::from("api_key_tpd:window"),
        expected_window_start: String::from("2026-11-27T00:00:00Z"),
        reservation_backend: BatchReservationBackend::Local,
    }
}

pub(crate) fn reservation(tokens: i64) -> BatchEnqueuedTokenReservation {
    BatchEnqueuedTokenReservation {
        tokens,
        scopes: vec![key_scope()],
        backend: ReservationBackend::Memory,
        owner: OPERATOR.to_owned(),
        reserved_at_monotonic: 0.0,
    }
}

pub(crate) fn batch_response() -> Json {
    let mut response = IndexMap::new();
    response.insert(String::from("id"), Json::Text(String::from("batch-1")));
    response.insert(
        String::from("status"),
        Json::Text(String::from("in_progress")),
    );
    Json::Object(response)
}

pub(crate) fn envelope(timestamp: Option<f64>, response: Json) -> Json {
    let mut fields = IndexMap::new();
    if let Some(seconds) = timestamp {
        fields.insert(
            String::from("timestamp"),
            Json::Number(Number::from_f64(seconds).expect("the timestamp is finite")),
        );
    }
    fields.insert(String::from("response"), response);
    Json::Object(fields)
}

pub(crate) fn namespaced() -> Json {
    namespaced_as(CACHE_NAME)
}

pub(crate) fn namespaced_as(name: &str) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("redis_namespace"), Json::Text(name.to_owned()));
    Json::Object(fields)
}

pub(crate) fn create_input(
    call_type: &str,
    principal: BatchLimitPrincipal,
    data: RequestData,
) -> BatchPrecallInput {
    BatchPrecallInput {
        call: BatchPrecallCall::Create(BatchCreateCall {
            call_type: call_type.to_owned(),
            principal,
            data,
        }),
    }
}

pub(crate) fn retrieve_input(cache_key: Option<&str>, max_age: Option<i64>) -> BatchPrecallInput {
    BatchPrecallInput {
        call: BatchPrecallCall::Retrieve(BatchRetrieveCall {
            call_type: RETRIEVE_CALL_TYPE.to_owned(),
            principal: plain_principal(),
            data: retrieve_data(),
            cache_key: cache_key.map(str::to_owned),
            cache_control: max_age.map(|value| BatchCacheControl {
                max_age: Some(value),
            }),
        }),
    }
}

pub(crate) fn retrieve_input_with_metadata(metadata: Option<Json>) -> BatchPrecallInput {
    let mut data = RequestData::new();
    data.insert(
        String::from("batch_id"),
        Json::Text(String::from("batch-1")),
    );
    if let Some(value) = metadata {
        data.insert(String::from("metadata"), value);
    }
    BatchPrecallInput {
        call: BatchPrecallCall::Retrieve(BatchRetrieveCall {
            call_type: RETRIEVE_CALL_TYPE.to_owned(),
            principal: plain_principal(),
            data,
            cache_key: None,
            cache_control: None,
        }),
    }
}

pub(crate) fn terminal_input(batch_id: &str, status: &str) -> BatchPrecallInput {
    BatchPrecallInput {
        call: BatchPrecallCall::TerminalRetrieve(BatchTerminalCall {
            batch_id: batch_id.to_owned(),
            status: status.to_owned(),
        }),
    }
}

pub(crate) fn failure_input() -> BatchPrecallInput {
    BatchPrecallInput {
        call: BatchPrecallCall::CreateFailure,
    }
}

pub(crate) fn debug_line(message: &str) -> BatchDiagnostic {
    BatchDiagnostic {
        level: BatchDiagnosticLevel::Debug,
        message: message.to_owned(),
    }
}

pub(crate) fn error_line(message: String) -> BatchDiagnostic {
    BatchDiagnostic {
        level: BatchDiagnosticLevel::Error,
        message,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    Skip {
        has_enqueued_scopes: bool,
    },
    Account {
        file_id: String,
        provider: String,
        descriptors: Option<Vec<BatchLimitDescriptor>>,
    },
    Reserve {
        tokens: i64,
        scopes: Vec<BatchEnqueuedTokenScope>,
    },
    Charge {
        tokens: i64,
        descriptors: Option<Vec<BatchLimitDescriptor>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageName {
    Skip,
    Account,
    Reserve,
    Charge,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted stage {stage:?} failed")]
pub(crate) struct StagesError {
    stage: StageName,
    http: Option<BatchHttpError>,
}

pub(crate) struct ScriptedStages {
    pub(crate) skip: BatchPrecallSkip,
    pub(crate) usage: BatchFileUsage,
    pub(crate) rejection: Option<Box<ProxyRateLimitError>>,
    pub(crate) fail_at: Option<StageName>,
    pub(crate) http_status: Option<u16>,
    effects: Mutex<Vec<Effect>>,
}

impl ScriptedStages {
    pub(crate) fn new() -> Self {
        Self {
            skip: BatchPrecallSkip {
                skip: false,
                descriptors: Some(vec![descriptor()]),
            },
            usage: file_usage(120, 2),
            rejection: None,
            fail_at: None,
            http_status: None,
            effects: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn effects(&self) -> Vec<Effect> {
        self.effects.lock().unwrap().clone()
    }

    fn record(&self, effect: Effect) {
        self.effects.lock().unwrap().push(effect);
    }

    fn fail(&self, stage: StageName) -> Result<(), StagesError> {
        if self.fail_at != Some(stage) {
            return Ok(());
        }
        Err(StagesError {
            stage,
            http: self.http_status.map(|status| BatchHttpError {
                status,
                detail: Json::Text(String::from("the batch file was rejected")),
            }),
        })
    }

    fn stage_answer(
        &self,
        effect: Effect,
        stage: StageName,
    ) -> Result<Option<Box<ProxyRateLimitError>>, StagesError> {
        self.record(effect);
        self.fail(stage)?;
        Ok(self.rejection.clone())
    }
}

impl BatchPrecallStages for ScriptedStages {
    type Error = StagesError;

    fn should_skip(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        has_enqueued_scopes: bool,
    ) -> Result<BatchPrecallSkip, StagesError> {
        self.record(Effect::Skip {
            has_enqueued_scopes,
        });
        self.fail(StageName::Skip)?;
        Ok(self.skip.clone())
    }

    fn account(
        &self,
        _principal: &BatchLimitPrincipal,
        request: &BatchAccountRequest<'_>,
    ) -> impl Future<Output = Result<BatchFileUsage, StagesError>> + Send {
        self.record(Effect::Account {
            file_id: request.file_id.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
            descriptors: request.descriptors.map(<[_]>::to_vec),
        });
        if let Err(failure) = self.fail(StageName::Account) {
            return ready(Err(failure));
        }
        ready(Ok(self.usage.clone()))
    }

    fn reserve(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StagesError>> + Send {
        ready(self.stage_answer(
            Effect::Reserve {
                tokens: usage.total_tokens,
                scopes: scopes.to_vec(),
            },
            StageName::Reserve,
        ))
    }

    fn charge(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        usage: &BatchFileUsage,
        descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> Ready<Result<Option<Box<ProxyRateLimitError>>, StagesError>> {
        let effect = Effect::Charge {
            tokens: usage.total_tokens,
            descriptors: descriptors.map(<[_]>::to_vec),
        };
        ready(self.stage_answer(effect, StageName::Charge))
    }

    fn http(&self, error: &StagesError) -> Option<BatchHttpError> {
        error.http.clone()
    }
}

#[derive(Default)]
pub(crate) struct ScriptedStash {
    reservation: Mutex<Option<BatchEnqueuedTokenReservation>>,
    operations: Mutex<Vec<BatchTpdRefundOperation>>,
}

impl ScriptedStash {
    pub(crate) fn holding(
        reservation: BatchEnqueuedTokenReservation,
        operations: Vec<BatchTpdRefundOperation>,
    ) -> Self {
        Self {
            reservation: Mutex::new(Some(reservation)),
            operations: Mutex::new(operations),
        }
    }

    pub(crate) fn reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        self.reservation.lock().unwrap().clone()
    }

    pub(crate) fn operations(&self) -> Vec<BatchTpdRefundOperation> {
        self.operations.lock().unwrap().clone()
    }
}

impl BatchPrecallStash for ScriptedStash {
    fn take_reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        self.reservation.lock().unwrap().take()
    }

    fn take_refund_operations(&self) -> Vec<BatchTpdRefundOperation> {
        mem::take(&mut *self.operations.lock().unwrap())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ApplierError {
    message: String,
}

#[derive(Default)]
pub(crate) struct ScriptedApplier {
    pub(crate) failure: Option<String>,
    applied: Mutex<Vec<Vec<BatchTpdRefundOperation>>>,
}

impl ScriptedApplier {
    pub(crate) fn applied(&self) -> Vec<Vec<BatchTpdRefundOperation>> {
        self.applied.lock().unwrap().clone()
    }
}

impl BatchTokenApplier for ScriptedApplier {
    type Error = ApplierError;

    fn apply(
        &self,
        operations: &[BatchTpdRefundOperation],
    ) -> impl Future<Output = Result<(), ApplierError>> + Send {
        self.applied.lock().unwrap().push(operations.to_vec());
        ready(self.failure.as_ref().map_or_else(
            || Ok(()),
            |message| {
                Err(ApplierError {
                    message: message.clone(),
                })
            },
        ))
    }
}

#[derive(Default)]
pub(crate) struct ScriptedMemory {
    pub(crate) values: Mutex<BTreeMap<String, Json>>,
    pub(crate) get_failure: Option<String>,
    pub(crate) set_failure: Option<String>,
    pub(crate) set_many_failure: Option<String>,
    sets: Mutex<Vec<(String, Json, f64)>>,
    pipelines: Mutex<Vec<(Vec<String>, f64)>>,
}

impl ScriptedMemory {
    pub(crate) fn store(&self, key: &str, value: Json) {
        self.values.lock().unwrap().insert(key.to_owned(), value);
    }

    fn set_entries(&self) -> Vec<(String, Json, f64)> {
        self.sets.lock().unwrap().clone()
    }

    fn pipeline_entries(&self) -> Vec<(Vec<String>, f64)> {
        self.pipelines.lock().unwrap().clone()
    }

    pub(crate) fn set_keys(&self) -> Vec<String> {
        self.set_entries()
            .into_iter()
            .map(|(key, _, _)| key)
            .collect()
    }

    pub(crate) fn set_ttls(&self) -> Vec<f64> {
        self.set_entries()
            .into_iter()
            .map(|(_, _, ttl)| ttl)
            .collect()
    }

    pub(crate) fn pipeline_keys(&self) -> Vec<Vec<String>> {
        self.pipeline_entries()
            .into_iter()
            .map(|(keys, _)| keys)
            .collect()
    }

    pub(crate) fn pipeline_ttls(&self) -> Vec<f64> {
        self.pipeline_entries()
            .into_iter()
            .map(|(_, ttl)| ttl)
            .collect()
    }
}

impl BatchPrecallMemory for ScriptedMemory {
    fn keys(&self) -> Vec<String> {
        self.values.lock().unwrap().keys().cloned().collect()
    }

    fn get(&self, key: &str) -> Result<Option<Json>, BatchCacheError> {
        if let Some(message) = &self.get_failure {
            return Err(BatchCacheError::Failed {
                message: message.clone(),
            });
        }
        Ok(self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned())
    }

    fn set(
        &self,
        key: &str,
        value: Json,
        ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        if let Some(message) = &self.set_failure {
            return ready(Err(BatchCacheError::Failed {
                message: message.clone(),
            }));
        }
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.clone());
        self.sets
            .lock()
            .unwrap()
            .push((key.to_owned(), value, ttl_seconds));
        ready(Ok(()))
    }

    fn set_many(
        &self,
        entries: Vec<(String, Json)>,
        ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        if let Some(message) = &self.set_many_failure {
            return ready(Err(BatchCacheError::Failed {
                message: message.clone(),
            }));
        }
        let keys = entries.iter().map(|(key, _value)| key.clone()).collect();
        {
            let mut values = self.values.lock().unwrap();
            for (key, value) in entries {
                values.insert(key, value);
            }
        }
        self.pipelines.lock().unwrap().push((keys, ttl_seconds));
        ready(Ok(()))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RemoteAnswer<T> {
    Value(T),
    Failed(String),
    Http { status: u16, detail: Json },
}

pub(crate) struct ScriptedRemote {
    pub(crate) redis: bool,
    pub(crate) key: Option<String>,
    pub(crate) scan: RemoteAnswer<Vec<String>>,
    pub(crate) batch_get: RemoteAnswer<Vec<(String, Json)>>,
    pub(crate) get: RemoteAnswer<Option<Json>>,
    scans: Mutex<Vec<(String, usize)>>,
    batches: Mutex<Vec<Vec<String>>>,
    gets: Mutex<Vec<String>>,
}

impl Default for ScriptedRemote {
    fn default() -> Self {
        Self {
            redis: true,
            key: None,
            scan: RemoteAnswer::Value(Vec::new()),
            batch_get: RemoteAnswer::Value(Vec::new()),
            get: RemoteAnswer::Value(None),
            scans: Mutex::new(Vec::new()),
            batches: Mutex::new(Vec::new()),
            gets: Mutex::new(Vec::new()),
        }
    }
}

impl ScriptedRemote {
    pub(crate) fn scan_patterns(&self) -> Vec<String> {
        self.scans
            .lock()
            .unwrap()
            .iter()
            .map(|(pattern, _count)| pattern.clone())
            .collect()
    }

    pub(crate) fn scan_counts(&self) -> Vec<usize> {
        self.scans
            .lock()
            .unwrap()
            .iter()
            .map(|(_pattern, count)| *count)
            .collect()
    }

    pub(crate) fn batch_keys(&self) -> Vec<Vec<String>> {
        self.batches.lock().unwrap().clone()
    }

    pub(crate) fn hits(&self) -> Vec<String> {
        self.gets.lock().unwrap().clone()
    }
}

fn resolved<T>(answer: &RemoteAnswer<T>) -> Result<T, BatchCacheError>
where
    T: Clone,
{
    match answer {
        RemoteAnswer::Value(value) => Ok(value.clone()),
        RemoteAnswer::Failed(message) => Err(BatchCacheError::Failed {
            message: message.clone(),
        }),
        RemoteAnswer::Http { status, detail } => Err(BatchCacheError::Http {
            status: *status,
            detail: detail.clone(),
        }),
    }
}

impl BatchPrecallRemote for ScriptedRemote {
    fn is_redis(&self) -> bool {
        self.redis
    }

    fn cache_key(&self, _data: &RequestData) -> Option<String> {
        self.key.clone()
    }

    fn scan(
        &self,
        pattern: &str,
        count: usize,
    ) -> impl Future<Output = Result<Vec<String>, BatchCacheError>> + Send {
        self.scans.lock().unwrap().push((pattern.to_owned(), count));
        ready(resolved(&self.scan))
    }

    fn batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Result<Vec<(String, Json)>, BatchCacheError>> + Send {
        self.batches.lock().unwrap().push(keys.to_vec());
        ready(resolved(&self.batch_get))
    }

    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Json>, BatchCacheError>> + Send {
        self.gets.lock().unwrap().push(key.to_owned());
        ready(resolved(&self.get))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test store has no Redis scripts")]
pub(crate) struct NoScriptsError;

pub(crate) struct NoScripts;

// the store without Lua scripts answers every script call with the same typed failure, already resolved
type NoScriptsReply<T> = Ready<Result<T, NoScriptsError>>;

impl ReservationScripts for NoScripts {
    type Error = NoScriptsError;

    fn reserve(
        &self,
        _counter_key: &str,
        _tokens: i64,
        _ttl_seconds: i64,
        _limit: i64,
    ) -> NoScriptsReply<ReserveScriptReply> {
        ready(Err(NoScriptsError))
    }

    fn refund(&self, _counter_key: &str, _tokens: i64) -> NoScriptsReply<()> {
        ready(Err(NoScriptsError))
    }
}

impl ReservationRecordScripts for NoScripts {
    fn save(&self, _record_key: &str, _record: &str, _ttl_seconds: i64) -> NoScriptsReply<()> {
        ready(Err(NoScriptsError))
    }

    fn pop(&self, _record_key: &str, _ttl_seconds: i64) -> NoScriptsReply<Option<Vec<u8>>> {
        ready(Err(NoScriptsError))
    }
}

pub(crate) struct Rig {
    pub(crate) stages: ScriptedStages,
    pub(crate) stash: ScriptedStash,
    pub(crate) counters: Arc<Mutex<LocalTokenCounters>>,
    pub(crate) applier: ScriptedApplier,
    pub(crate) memory: ScriptedMemory,
    pub(crate) remote: ScriptedRemote,
    pub(crate) clock: EpochClock,
    pub(crate) limits: ApiLimits,
    pub(crate) verbose_stdout: bool,
}

impl Rig {
    pub(crate) fn new() -> Self {
        let cache = LocalTtlCache::with_clock(None, None, None, 1_024, Arc::new(|| 0.0_f64))
            .expect("the local cache builds");
        let counters = Arc::new(Mutex::new(LocalTokenCounters { cache }));
        let clock: EpochClock = Arc::new(|| 100.0_f64);
        let stages = ScriptedStages::new();
        Self {
            stages,
            stash: ScriptedStash::default(),
            counters,
            applier: ScriptedApplier::default(),
            memory: ScriptedMemory::default(),
            remote: ScriptedRemote::default(),
            clock,
            limits: ApiLimits::default(),
            verbose_stdout: false,
        }
    }

    pub(crate) async fn run(
        &self,
        input: &BatchPrecallInput,
    ) -> Result<BatchPrecallResult, BatchPrecallError> {
        self.run_with(input, Some(&self.remote)).await
    }

    pub(crate) async fn run_without_remote(
        &self,
        input: &BatchPrecallInput,
    ) -> Result<BatchPrecallResult, BatchPrecallError> {
        self.run_with(input, None).await
    }

    async fn run_with(
        &self,
        input: &BatchPrecallInput,
        remote: Option<&ScriptedRemote>,
    ) -> Result<BatchPrecallResult, BatchPrecallError> {
        let refunds: EnqueuedTokenRefund<NoScripts, LocalTokenCounters> = EnqueuedTokenRefund {
            clock: Arc::new(|| 100.0_f64),
            scripts: None,
            store: Arc::clone(&self.counters),
            owner: OPERATOR.to_owned(),
            settings: BatchEnqueuedTokenSettings::default(),
            limits: ApiLimits::default(),
        };
        let service = BatchPrecall {
            stages: &self.stages,
            stash: &self.stash,
            refunds: &refunds,
            applier: &self.applier,
            memory: &self.memory,
            remote,
            verbose_stdout: self.verbose_stdout,
            clock: Arc::clone(&self.clock),
            limits: self.limits,
        };
        service.run(input).await
    }
}

pub(crate) fn seed_counter(counters: &Arc<Mutex<LocalTokenCounters>>, value: i64) {
    let mut held = counters.lock().unwrap();
    EnqueuedTokenCounters::set(&mut *held, COUNTER_KEY, value, 691_200).expect("the counter seeds");
}

pub(crate) fn stored_counter(counters: &Arc<Mutex<LocalTokenCounters>>) -> Option<i64> {
    let mut held = counters.lock().unwrap();
    EnqueuedTokenCounters::get(&mut *held, COUNTER_KEY).expect("the counter reads")
}

pub(crate) fn break_counter(counters: &Arc<Mutex<LocalTokenCounters>>) {
    let mut held = counters.lock().unwrap();
    held.cache
        .set(
            COUNTER_KEY,
            CacheValue::Text(String::from("not a counter")),
            None,
        )
        .expect("the broken counter stores");
}

pub(crate) fn record_key(batch_id: &str) -> String {
    format!("batch_enqueued_token_reservation:{batch_id}")
}

pub(crate) fn stored_record(
    counters: &Arc<Mutex<LocalTokenCounters>>,
    batch_id: &str,
) -> Option<Vec<u8>> {
    let mut held = counters.lock().unwrap();
    held.record(&record_key(batch_id))
        .expect("the record reads")
}
