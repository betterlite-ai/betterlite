use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use betterlite_redact::structured::Payload;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedReserveResult, BatchEnqueuedTokenOverLimit, BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope, ReservationBackend, ScopeKey,
};
use crate::api::batch_limit_counters::BatchLimitCounters;
use crate::api::batch_limit_counters::input::{BatchLimitCounterCall, BatchLimitCountersInput};
use crate::api::batch_limit_counters::ports::{
    BatchCounterLimiter, BatchCounterStash, BatchEnqueuedSource, BatchWindowCache,
};
use crate::api::batch_limit_counters::records::{
    BatchCounterCode, BatchCounterLimitType, BatchCounterResponse, BatchCounterStatus,
    BatchEnqueuedLimitExceeded, BatchFileUsage, BatchLimitCounterAnswer, BatchReservationBackend,
    BatchReservationWindow, BatchTpdRefundOperation, WindowStartValue,
};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchDescriptorRateLimit, BatchLimitDescriptor,
};
use crate::api::batch_limit_descriptors::input::{BatchLimitPrincipal, BatchModelUsage};
use crate::api::request_destination::RequestData;

const NOW: f64 = 1_700_000_000.0;
pub(super) const WINDOW: i64 = 60;
pub(super) const RESET_AT: &str = "2023-11-14 22:14:20 UTC";

#[derive(Debug, thiserror::Error)]
#[error("the scripted limiter failed")]
struct LimiterFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted store failed")]
struct StoreFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted cache failed")]
struct CacheFailure;

#[derive(Clone, Default)]
pub(super) struct ScriptedLimiter {
    pub(super) base: Vec<BatchLimitDescriptor>,
    pub(super) project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    pub(super) responses: Arc<Mutex<VecDeque<Result<BatchCounterResponse, LimiterFailure>>>>,
    pub(super) calls: Arc<Mutex<Vec<(Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>)>>>,
    pub(super) base_calls: Arc<Mutex<u32>>,
    project_usages: Arc<Mutex<Vec<IndexMap<String, BatchModelUsage>>>>,
}

impl ScriptedLimiter {
    pub(super) fn with_responses(
        responses: Vec<Result<BatchCounterResponse, LimiterFailure>>,
    ) -> Self {
        Self {
            base: Vec::new(),
            project: (Vec::new(), Vec::new()),
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            calls: Arc::default(),
            base_calls: Arc::default(),
            project_usages: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<(Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }

    pub(super) fn project_usages(&self) -> Vec<IndexMap<String, BatchModelUsage>> {
        let usages = self
            .project_usages
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        usages.clone()
    }

    pub(super) fn base_calls(&self) -> u32 {
        *self
            .base_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl BatchCounterLimiter for ScriptedLimiter {
    type Error = LimiterFailure;

    fn base_descriptors(
        &self,
        _auth: &UserAPIKeyAuth,
        _data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, LimiterFailure> {
        let mut count = self
            .base_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_add(1);
        drop(count);
        Ok(self.base.clone())
    }

    fn project_io_descriptors(
        &self,
        _auth: &UserAPIKeyAuth,
        per_model_usage: &IndexMap<String, BatchModelUsage>,
    ) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>) {
        self.project_usages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(per_model_usage.clone());
        self.project.clone()
    }

    fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[BatchLimitDescriptor],
        increments: &[BatchDescriptorIncrements],
    ) -> impl Future<Output = Result<BatchCounterResponse, LimiterFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((descriptors.to_vec(), increments.to_vec()));
        let reply = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(reply.unwrap_or_else(|| Ok(ok_response(Vec::new(), Vec::new()))))
    }

    fn create_rate_limit_keys(
        &self,
        key: &str,
        value: &str,
        rate_limit_type: BatchCounterLimitType,
    ) -> String {
        format!("{{{key}:{value}}}:{}", rate_limit_type.as_str())
    }
}

#[derive(Clone, Default)]
pub(super) struct ScriptedStore {
    replies: Arc<Mutex<Vec<Result<BatchEnqueuedReserveResult, StoreFailure>>>>,
    calls: Arc<Mutex<Vec<(i64, Vec<BatchEnqueuedTokenScope>)>>>,
}

impl ScriptedStore {
    pub(super) fn with_reply(reply: Result<BatchEnqueuedReserveResult, StoreFailure>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(vec![reply])),
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<(i64, Vec<BatchEnqueuedTokenScope>)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl BatchEnqueuedSource for ScriptedStore {
    type Error = StoreFailure;

    fn reserve(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<BatchEnqueuedReserveResult, StoreFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((tokens, scopes.to_vec()));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        ready(reply.unwrap_or_else(|| Ok(reserved(tokens, scopes))))
    }
}

#[derive(Clone, Default)]
pub(super) struct ScriptedCache {
    pub(super) values: BTreeMap<String, WindowStartValue>,
    pub(super) reads: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCache {
    pub(super) fn with_value(key: &str, value: WindowStartValue) -> Self {
        Self {
            values: BTreeMap::from([(String::from(key), value)]),
            reads: Arc::default(),
        }
    }

    pub(super) fn reads(&self) -> Vec<String> {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.clone()
    }
}

impl BatchWindowCache for ScriptedCache {
    type Error = CacheFailure;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<WindowStartValue>, CacheFailure>> + Send {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        ready(Ok(self.values.get(key).cloned()))
    }
}

#[derive(Default)]
pub(super) struct RecordingStash {
    reservation: Mutex<Option<BatchEnqueuedTokenReservation>>,
    operations: Mutex<Vec<BatchTpdRefundOperation>>,
}

impl RecordingStash {
    pub(super) fn reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        let stored = self
            .reservation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stored.clone()
    }

    pub(super) fn operations(&self) -> Vec<BatchTpdRefundOperation> {
        let stored = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stored.clone()
    }
}

impl BatchCounterStash for RecordingStash {
    fn set_batch_enqueued_reservation(&self, reservation: BatchEnqueuedTokenReservation) {
        let mut stored = self
            .reservation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *stored = Some(reservation);
    }

    fn set_batch_tpd_refund_ops(&self, operations: Vec<BatchTpdRefundOperation>) {
        let mut stored = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *stored = operations;
    }
}

struct ScriptedResolver {
    models: BTreeMap<String, String>,
}

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        let Some(provider) = self.models.get(model) else {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        };
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Custom(provider.clone()),
            api_base: None,
            api_key: None,
        })
    }
}

pub(super) struct Harness {
    pub(super) service: BatchLimitCounters<ScriptedLimiter, ScriptedStore, ScriptedCache>,
    pub(super) stash: Arc<RecordingStash>,
    pub(super) limiter: ScriptedLimiter,
    pub(super) store: ScriptedStore,
    pub(super) cache: ScriptedCache,
}

pub(super) fn harness(
    limiter: ScriptedLimiter,
    store: ScriptedStore,
    cache: ScriptedCache,
) -> Harness {
    let stash = Arc::new(RecordingStash::default());
    let service = BatchLimitCounters {
        clock: Arc::new(|| NOW),
        limiter: limiter.clone(),
        store: store.clone(),
        cache: cache.clone(),
        stash: stash.clone(),
        resolver: Arc::new(ScriptedResolver {
            models: BTreeMap::from([(String::from("gpt-4o-mini"), String::from("openai"))]),
        }),
        router: None,
        window_size: WINDOW,
        limits: ApiLimits::default(),
    };
    Harness {
        service,
        stash,
        limiter,
        store,
        cache,
    }
}

pub(super) fn data(model: Option<&str>) -> RequestData {
    let mut fields = RequestData::new();
    if let Some(name) = model {
        fields.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    fields
}

pub(super) fn null_model_data() -> RequestData {
    let mut fields = RequestData::new();
    fields.insert(String::from("model"), Json::Null);
    fields
}

pub(super) fn usage(total_tokens: i64, request_count: i64, output_tokens: i64) -> BatchFileUsage {
    BatchFileUsage {
        total_tokens,
        request_count,
        output_tokens,
        per_model_usage: IndexMap::new(),
    }
}

pub(super) fn descriptor(
    key: &str,
    value: &str,
    window_size: Option<i64>,
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
) -> BatchLimitDescriptor {
    BatchLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(BatchDescriptorRateLimit {
            requests_per_unit,
            tokens_per_unit,
            max_parallel_requests: None,
            window_size,
        }),
    }
}

pub(super) fn status(
    code: BatchCounterCode,
    descriptor_key: &str,
    descriptor_value: Option<&str>,
    rate_limit_type: BatchCounterLimitType,
    current_limit: i64,
    limit_remaining: i64,
) -> BatchCounterStatus {
    BatchCounterStatus {
        code,
        current_limit,
        limit_remaining,
        rate_limit_type,
        descriptor_key: descriptor_key.to_owned(),
        descriptor_value: descriptor_value.map(str::to_owned),
    }
}

pub(super) fn response(
    overall_code: BatchCounterCode,
    statuses: Vec<BatchCounterStatus>,
    windows: Vec<(&str, &str, BatchReservationBackend)>,
) -> BatchCounterResponse {
    BatchCounterResponse {
        overall_code,
        statuses,
        reservation_windows: windows
            .into_iter()
            .map(
                |(counter_key, window_start, backend)| BatchReservationWindow {
                    counter_key: counter_key.to_owned(),
                    window_start: window_start.to_owned(),
                    backend,
                },
            )
            .collect(),
    }
}

pub(super) fn ok_response(
    statuses: Vec<BatchCounterStatus>,
    windows: Vec<(&str, &str, BatchReservationBackend)>,
) -> BatchCounterResponse {
    response(BatchCounterCode::Ok, statuses, windows)
}

pub(super) fn reserved(
    tokens: i64,
    scopes: &[BatchEnqueuedTokenScope],
) -> BatchEnqueuedReserveResult {
    BatchEnqueuedReserveResult::Reservation(BatchEnqueuedTokenReservation {
        tokens,
        scopes: scopes.to_vec(),
        backend: ReservationBackend::Memory,
        owner: String::new(),
        reserved_at_monotonic: 0.0,
    })
}

pub(super) fn over_limit(
    scope: BatchEnqueuedTokenScope,
    enqueued: i64,
) -> BatchEnqueuedReserveResult {
    BatchEnqueuedReserveResult::OverLimit(BatchEnqueuedTokenOverLimit { scope, enqueued })
}

pub(super) fn scope(value: &str, limit: i64) -> BatchEnqueuedTokenScope {
    BatchEnqueuedTokenScope {
        key: ScopeKey::ApiKey,
        value: value.to_owned(),
        limit,
    }
}

pub(super) fn window(
    counter_key: &str,
    window_start: &str,
    backend: BatchReservationBackend,
) -> BatchReservationWindow {
    BatchReservationWindow {
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        backend,
    }
}

pub(super) fn input(calls: Vec<BatchLimitCounterCall>) -> BatchLimitCountersInput {
    BatchLimitCountersInput {
        principal: BatchLimitPrincipal(BTreeMap::new()),
        calls,
    }
}

// a call that answers without rejecting and touches no dependency
pub(super) fn refund_call() -> BatchLimitCounterCall {
    BatchLimitCounterCall::BuildTpdRefundOps {
        descriptors: Vec::new(),
        tokens: 0,
        reservation_windows: BTreeSet::new(),
    }
}

pub(super) fn detail_text(value: &Payload) -> &str {
    match value {
        Payload::String(text) => text,
        other @ (Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Object(_)
        | Payload::Array(_)) => panic!("the detail is not text: {other:?}"),
    }
}

pub(super) fn header(error: &ProxyRateLimitError, name: &str) -> String {
    let headers = error.headers.as_ref().expect("a rejection carries headers");
    let value = headers.get(name).expect("the named header is present");
    value.to_str().expect("the header is text").to_owned()
}

pub(super) fn rejection(answer: &BatchLimitCounterAnswer) -> &ProxyRateLimitError {
    match answer {
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters {
            rejection: Some(error),
        }
        | BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens {
            rejection: Some(error),
        }
        | BatchLimitCounterAnswer::RaiseEnqueuedLimitError { error }
        | BatchLimitCounterAnswer::RaiseRateLimitError { error } => error,
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None }
        | BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection: None }
        | BatchLimitCounterAnswer::ReadTpdWindowStart { .. }
        | BatchLimitCounterAnswer::BuildTpdRefundOps { .. } => {
            panic!("the answer is not a rejection")
        }
    }
}

pub(super) async fn assert_direct_enqueued_model(
    harness: &Harness,
    fields: RequestData,
    expected_model: &str,
    expected_provider: &str,
) {
    let answer = harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::RaiseEnqueuedLimitError {
                over_limit: BatchEnqueuedLimitExceeded {
                    scope: scope("sk-1", 15),
                    enqueued: 10,
                },
                data: fields,
                batch_usage: usage(30, 2, 0),
            },
        ]))
        .await
        .unwrap();
    let error = rejection(&answer.answers[0]);
    assert_eq!(error.model, expected_model);
    assert_eq!(error.llm_provider, expected_provider);
}

#[derive(Clone, Copy)]
pub(super) enum StoreReplyKind {
    Reservation,
    OverLimit,
}

#[derive(Clone, Copy)]
pub(super) enum Stored {
    Integer(i64),
    Float(f64),
    Text(&'static str),
}

pub(super) fn stored_value(stored: Stored) -> WindowStartValue {
    match stored {
        Stored::Integer(value) => WindowStartValue::Integer(value),
        Stored::Float(value) => WindowStartValue::Float(value),
        Stored::Text(text) => WindowStartValue::Text(text.to_owned()),
    }
}
