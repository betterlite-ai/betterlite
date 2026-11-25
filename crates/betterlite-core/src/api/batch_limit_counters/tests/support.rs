use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::rate_limit::{ProxyRateLimitError, RateLimitCategory, RateLimitDimension};
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
    BatchEnqueuedLimitExceeded, BatchFileUsage, BatchLimitCounterAnswer, BatchLimitCountersResult,
    BatchReservationBackend, BatchReservationWindow, BatchTpdRefundOperation, WindowStartValue,
};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchDescriptorRateLimit, BatchLimitDescriptor, TPD_WINDOW_SECONDS,
};
use crate::api::batch_limit_descriptors::input::{BatchLimitPrincipal, BatchModelUsage};
use crate::api::request_destination::RequestData;

const NOW: f64 = 1_700_000_000.0;
pub(super) const WINDOW: i64 = 60;
pub(super) const RESET_AT: &str = "2023-11-14 22:14:20 UTC";
const ENQUEUED_DETAIL: &str =
    "batch enqueued token limit reached for api_key: sk-1: 30 tokens requested, 5 of 15 remaining";

pub(super) type LimiterCall = (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>);
pub(super) type StoreCall = (i64, Vec<BatchEnqueuedTokenScope>);

#[derive(Debug, thiserror::Error)]
#[error("the scripted limiter failed")]
pub(super) struct LimiterFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted store failed")]
pub(super) struct StoreFailure;

#[derive(Debug, thiserror::Error)]
#[error("the scripted cache failed")]
pub(super) struct CacheFailure;

#[derive(Clone, Default)]
pub(super) struct ScriptedLimiter {
    pub(super) base: Vec<BatchLimitDescriptor>,
    pub(super) project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    pub(super) responses: Arc<Mutex<VecDeque<Result<BatchCounterResponse, LimiterFailure>>>>,
    pub(super) calls: Arc<Mutex<Vec<LimiterCall>>>,
    pub(super) base_calls: Arc<Mutex<u32>>,
    pub(super) project_usages: Arc<Mutex<Vec<IndexMap<String, BatchModelUsage>>>>,
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

    pub(super) fn calls(&self) -> Vec<LimiterCall> {
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
    calls: Arc<Mutex<Vec<StoreCall>>>,
}

impl ScriptedStore {
    pub(super) fn with_reply(reply: Result<BatchEnqueuedReserveResult, StoreFailure>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(vec![reply])),
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<StoreCall> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl BatchEnqueuedSource for ScriptedStore {
    type Error = StoreFailure;

    fn reserve_tokens(
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
    let stash_handle: Arc<dyn BatchCounterStash> = Arc::<RecordingStash>::clone(&stash);
    let service = BatchLimitCounters {
        clock: Arc::new(|| NOW),
        limiter: limiter.clone(),
        store: store.clone(),
        cache: cache.clone(),
        stash: stash_handle,
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

pub(super) fn non_text_model_data() -> RequestData {
    let mut fields = RequestData::new();
    fields.insert(String::from("model"), Json::Bool(true));
    fields
}

pub(super) fn empty_model_data() -> RequestData {
    let mut fields = RequestData::new();
    fields.insert(String::from("model"), Json::Text(String::new()));
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
    (current_limit, limit_remaining): (i64, i64),
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

const DAILY_REFUND_WINDOW: (&str, &str, BatchReservationBackend) = (
    "{api_key_tpd:sk-1}:tokens",
    "1699990000",
    BatchReservationBackend::Redis,
);

pub(super) fn daily_refund_response() -> BatchCounterResponse {
    ok_response(Vec::new(), vec![DAILY_REFUND_WINDOW])
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

pub(super) const fn over_limit(
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

pub(super) const fn input(calls: Vec<BatchLimitCounterCall>) -> BatchLimitCountersInput {
    BatchLimitCountersInput {
        principal: BatchLimitPrincipal(BTreeMap::new()),
        calls,
    }
}

// a call that answers without rejecting and touches no dependency
pub(super) const fn refund_call() -> BatchLimitCounterCall {
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

pub(super) fn daily_descriptor() -> BatchLimitDescriptor {
    descriptor(
        "api_key_tpd",
        "sk-1",
        Some(TPD_WINDOW_SECONDS),
        None,
        Some(100),
    )
}

// the batch's own counts, plus the named model's row when a case charges one
pub(super) fn usage_for_row(
    model: Option<&str>,
    row_total: Option<i64>,
    row_output: Option<i64>,
) -> BatchFileUsage {
    let mut batch_usage = usage(120, 3, 7);
    if let Some(name) = model {
        let mut row = BTreeMap::new();
        if let Some(total) = row_total {
            row.insert(String::from("total_tokens"), total);
        }
        if let Some(output) = row_output {
            row.insert(String::from("output_tokens"), output);
        }
        batch_usage.per_model_usage.insert(name.to_owned(), row);
    }
    batch_usage
}

pub(super) fn usage_with_rows() -> BatchFileUsage {
    let mut batch_usage = usage(120, 3, 30);
    batch_usage.per_model_usage.insert(
        String::from("gpt-4o"),
        BTreeMap::from([(String::from("output_tokens"), 30)]),
    );
    batch_usage
}

pub(super) fn project_usage_row() -> IndexMap<String, BatchModelUsage> {
    IndexMap::from_iter([(
        String::from("gpt-4o"),
        BatchModelUsage {
            total_tokens: 0,
            output_tokens: 30,
        },
    )])
}

pub(super) const fn increments(requests: i64, tokens: i64) -> BatchDescriptorIncrements {
    BatchDescriptorIncrements { requests, tokens }
}

// a limiter that answers one requests-counter check with the given status
pub(super) fn requests_limiter(code: BatchCounterCode, remaining: i64) -> ScriptedLimiter {
    ScriptedLimiter::with_responses(vec![Ok(response(
        code,
        vec![status(
            code,
            "api_key",
            Some("sk-1"),
            BatchCounterLimitType::Requests,
            (5, remaining),
        )],
        Vec::new(),
    ))])
}

// three charges on the daily counter and two on counters that get no refund
pub(super) fn refund_windows() -> BTreeSet<BatchReservationWindow> {
    BTreeSet::from([
        window(
            "{api_key_tpd:sk-1}:tokens",
            "1699990000",
            BatchReservationBackend::Local,
        ),
        window(
            "{api_key_tpd:sk-1}:tokens",
            "1699990000",
            BatchReservationBackend::Redis,
        ),
        window(
            "{api_key_tpd:sk-1}:tokens",
            "1699990001",
            BatchReservationBackend::Local,
        ),
        window(
            "{api_key:sk-1}:tokens",
            "1699990000",
            BatchReservationBackend::Redis,
        ),
        window(
            "{team:team-1}:tokens",
            "1699990000",
            BatchReservationBackend::Redis,
        ),
    ])
}

// the refunds refund_windows produces, in the order the daily counter's windows sort
pub(super) fn expected_operations(increment_value: i64) -> Vec<BatchTpdRefundOperation> {
    let window_key = String::from("{api_key_tpd:sk-1}:window");
    [
        ("1699990000", BatchReservationBackend::Local),
        ("1699990000", BatchReservationBackend::Redis),
        ("1699990001", BatchReservationBackend::Local),
    ]
    .into_iter()
    .map(
        |(window_start, reservation_backend)| BatchTpdRefundOperation {
            key: String::from("{api_key_tpd:sk-1}:tokens"),
            increment_value,
            ttl: TPD_WINDOW_SECONDS,
            window_key: window_key.clone(),
            expected_window_start: String::from(window_start),
            reservation_backend,
        },
    )
    .collect()
}

#[derive(Clone, Copy)]
struct PartialFailureRow {
    code: BatchCounterCode,
    key: &'static str,
    value: &'static str,
    limit_type: BatchCounterLimitType,
    limits: (i64, i64),
}

// the statuses a partial failure reports: an Ok row, then the daily and team counters over their limits
const PARTIAL_FAILURE_ROWS: [PartialFailureRow; 3] = [
    PartialFailureRow {
        code: BatchCounterCode::Ok,
        key: "api_key",
        value: "sk-1",
        limit_type: BatchCounterLimitType::Requests,
        limits: (5, 1),
    },
    PartialFailureRow {
        code: BatchCounterCode::OverLimit,
        key: "api_key_tpd",
        value: "sk-1",
        limit_type: BatchCounterLimitType::Tokens,
        limits: (100, 0),
    },
    PartialFailureRow {
        code: BatchCounterCode::OverLimit,
        key: "team_tpd",
        value: "team-1",
        limit_type: BatchCounterLimitType::Tokens,
        limits: (100, 0),
    },
];

pub(super) const PARTIAL_FAILURE_DETAIL: &str =
    "batch tokens rate limit reached for api_key_tpd: sk-1: 120 tokens, 0 of 100 per day remaining";

fn row_status(row: PartialFailureRow) -> BatchCounterStatus {
    let PartialFailureRow {
        code,
        key,
        value,
        limit_type,
        limits,
    } = row;
    status(code, key, Some(value), limit_type, limits)
}

pub(super) fn partial_failure_statuses() -> Vec<BatchCounterStatus> {
    PARTIAL_FAILURE_ROWS.into_iter().map(row_status).collect()
}

// base and project counters that charge in one call, answered by partial_failure_statuses
pub(super) fn partial_failure_limiter() -> ScriptedLimiter {
    ScriptedLimiter {
        base: vec![
            descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None),
            daily_descriptor(),
        ],
        project: (
            vec![descriptor(
                "model_per_project_otpm",
                "proj:gpt-4o",
                Some(WINDOW),
                None,
                Some(100),
            )],
            vec![increments(0, 30)],
        ),
        ..ScriptedLimiter::with_responses(vec![Ok(response(
            BatchCounterCode::OverLimit,
            partial_failure_statuses(),
            Vec::new(),
        ))])
    }
}

pub(super) fn partial_failure_cache() -> ScriptedCache {
    ScriptedCache::with_value(
        "{api_key_tpd:sk-1}:window",
        WindowStartValue::Integer(1_699_990_000),
    )
}

pub(super) fn window_status(
    descriptor_key: &str,
    descriptor_value: Option<&str>,
) -> BatchCounterStatus {
    status(
        BatchCounterCode::OverLimit,
        descriptor_key,
        descriptor_value,
        BatchCounterLimitType::Tokens,
        (10, 0),
    )
}

pub(super) async fn run_check(
    limiter: ScriptedLimiter,
    cache: ScriptedCache,
    data: RequestData,
    batch_usage: BatchFileUsage,
    descriptors: Option<Vec<BatchLimitDescriptor>>,
) -> (Harness, BatchLimitCountersResult) {
    let harness = harness(limiter, ScriptedStore::default(), cache);
    let result = harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data,
                batch_usage,
                descriptors,
            },
        ]))
        .await
        .unwrap();
    (harness, result)
}

pub(super) async fn run_raise(
    status: BatchCounterStatus,
    descriptors: Vec<BatchLimitDescriptor>,
    batch_usage: BatchFileUsage,
    requested_model: Option<&str>,
    window_start: Option<i64>,
) -> (Harness, BatchLimitCountersResult) {
    let limit_type = status.rate_limit_type.as_str();
    let harness = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let result = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::RaiseRateLimitError {
            status,
            descriptors,
            batch_usage,
            limit_type: String::from(limit_type),
            requested_model: requested_model.map(str::to_owned),
            window_start,
        }]))
        .await
        .unwrap();
    (harness, result)
}

pub(super) async fn run_window(
    cache: ScriptedCache,
    status: BatchCounterStatus,
) -> (Harness, BatchLimitCountersResult) {
    let harness = harness(ScriptedLimiter::default(), ScriptedStore::default(), cache);
    let result = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::ReadTpdWindowStart {
            status,
        }]))
        .await
        .unwrap();
    (harness, result)
}

pub(super) async fn run_refund(
    descriptors: Vec<BatchLimitDescriptor>,
    tokens: i64,
    reservation_windows: BTreeSet<BatchReservationWindow>,
) -> (Harness, BatchLimitCountersResult) {
    let harness = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let result = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::BuildTpdRefundOps {
            descriptors,
            tokens,
            reservation_windows,
        }]))
        .await
        .unwrap();
    (harness, result)
}

pub(super) fn assert_requests_rejection(
    answer: &BatchLimitCounterAnswer,
    expected_model: &str,
    expected_provider: &str,
    expected_detail: &str,
    retry_after: &str,
) {
    let error = rejection(answer);
    assert_eq!(error.category, RateLimitCategory::LitellmBatchRateLimit);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
    assert_eq!(error.model, expected_model);
    assert_eq!(error.llm_provider, expected_provider);
    assert_eq!(detail_text(&error.detail), expected_detail);
    assert_eq!(header(error, "retry-after"), retry_after);
    assert_eq!(header(error, "rate_limit_type"), "requests");
    assert_eq!(header(error, "reset_at"), RESET_AT);
}

pub(super) fn assert_enqueued_rejection(
    answer: &BatchLimitCounterAnswer,
    expected_model: &str,
    expected_provider: &str,
) {
    let error = rejection(answer);
    assert_eq!(error.category, RateLimitCategory::LitellmBatchRateLimit);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(detail_text(&error.detail), ENQUEUED_DETAIL);
    assert_eq!(header(error, "rate_limit_type"), "tokens");
    assert_eq!(error.model, expected_model);
    assert_eq!(error.llm_provider, expected_provider);
}

pub(super) fn window_start_of(answer: &BatchLimitCounterAnswer) -> Option<i64> {
    let BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } = answer else {
        panic!("not a window answer: {answer:?}");
    };
    *window_start
}

pub(super) fn operations_of(answer: &BatchLimitCounterAnswer) -> &Vec<BatchTpdRefundOperation> {
    let BatchLimitCounterAnswer::BuildTpdRefundOps { operations } = answer else {
        panic!("not a refund answer: {answer:?}");
    };
    operations
}
