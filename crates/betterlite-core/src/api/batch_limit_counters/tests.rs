#![cfg(test)]

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
use rstest::rstest;

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedReserveResult, BatchEnqueuedTokenOverLimit, BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope, ReservationBackend, ScopeKey,
};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchDescriptorRateLimit, BatchLimitDescriptor, TPD_WINDOW_SECONDS,
};
use crate::api::batch_limit_descriptors::input::{BatchLimitPrincipal, BatchModelUsage};
use crate::api::request_destination::RequestData;

use super::input::{BatchLimitCounterCall, BatchLimitCountersInput};
use super::ports::{BatchCounterLimiter, BatchCounterStash, BatchEnqueuedSource, BatchWindowCache};
use super::records::{
    BatchCounterCode, BatchCounterLimitType, BatchCounterResponse, BatchCounterStatus,
    BatchEnqueuedLimitExceeded, BatchFileUsage, BatchLimitCounterAnswer, BatchReservationBackend,
    BatchReservationWindow, BatchTpdRefundOperation, WindowStartValue,
};
use super::{BatchLimitCounters, BatchLimitCountersError};

const NOW: f64 = 1_700_000_000.0;
const WINDOW: i64 = 60;
const RESET_AT: &str = "2023-11-14 22:14:20 UTC";

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
struct ScriptedLimiter {
    base: Vec<BatchLimitDescriptor>,
    project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    responses: Arc<Mutex<VecDeque<Result<BatchCounterResponse, LimiterFailure>>>>,
    calls: Arc<Mutex<Vec<(Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>)>>>,
    base_calls: Arc<Mutex<u32>>,
}

impl ScriptedLimiter {
    fn with_responses(responses: Vec<Result<BatchCounterResponse, LimiterFailure>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<(Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }

    fn base_calls(&self) -> u32 {
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
        _per_model_usage: &IndexMap<String, BatchModelUsage>,
    ) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>) {
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
struct ScriptedStore {
    replies: Arc<Mutex<VecDeque<Result<BatchEnqueuedReserveResult, StoreFailure>>>>,
    calls: Arc<Mutex<Vec<(i64, Vec<BatchEnqueuedTokenScope>)>>>,
}

impl ScriptedStore {
    fn with_replies(replies: Vec<Result<BatchEnqueuedReserveResult, StoreFailure>>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<(i64, Vec<BatchEnqueuedTokenScope>)> {
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
            .pop_front();
        ready(reply.unwrap_or_else(|| Ok(reserved(tokens, scopes))))
    }
}

#[derive(Clone, Default)]
struct ScriptedCache {
    values: BTreeMap<String, WindowStartValue>,
    reads: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCache {
    fn with_value(key: &str, value: WindowStartValue) -> Self {
        Self {
            values: BTreeMap::from([(String::from(key), value)]),
            ..Self::default()
        }
    }

    fn reads(&self) -> Vec<String> {
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
struct RecordingStash {
    reservation: Mutex<Option<BatchEnqueuedTokenReservation>>,
    operations: Mutex<Vec<BatchTpdRefundOperation>>,
}

impl RecordingStash {
    fn reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        let stored = self
            .reservation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stored.clone()
    }

    fn operations(&self) -> Vec<BatchTpdRefundOperation> {
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

struct Harness {
    service: BatchLimitCounters<ScriptedLimiter, ScriptedStore, ScriptedCache>,
    stash: Arc<RecordingStash>,
    limiter: ScriptedLimiter,
    store: ScriptedStore,
    cache: ScriptedCache,
}

fn harness(limiter: ScriptedLimiter, store: ScriptedStore, cache: ScriptedCache) -> Harness {
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

fn data(model: Option<&str>) -> RequestData {
    let mut fields = RequestData::new();
    if let Some(name) = model {
        fields.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    fields
}

fn null_model_data() -> RequestData {
    let mut fields = RequestData::new();
    fields.insert(String::from("model"), Json::Null);
    fields
}

fn usage(total_tokens: i64, request_count: i64, output_tokens: i64) -> BatchFileUsage {
    BatchFileUsage {
        total_tokens,
        request_count,
        output_tokens,
        per_model_usage: IndexMap::new(),
    }
}

fn descriptor(
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

fn status(
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

fn response(
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

fn ok_response(
    statuses: Vec<BatchCounterStatus>,
    windows: Vec<(&str, &str, BatchReservationBackend)>,
) -> BatchCounterResponse {
    response(BatchCounterCode::Ok, statuses, windows)
}

fn reserved(tokens: i64, scopes: &[BatchEnqueuedTokenScope]) -> BatchEnqueuedReserveResult {
    BatchEnqueuedReserveResult::Reservation(BatchEnqueuedTokenReservation {
        tokens,
        scopes: scopes.to_vec(),
        backend: ReservationBackend::Memory,
        owner: String::new(),
        reserved_at_monotonic: 0.0,
    })
}

fn over_limit(scope: BatchEnqueuedTokenScope, enqueued: i64) -> BatchEnqueuedReserveResult {
    BatchEnqueuedReserveResult::OverLimit(BatchEnqueuedTokenOverLimit { scope, enqueued })
}

fn scope(value: &str, limit: i64) -> BatchEnqueuedTokenScope {
    BatchEnqueuedTokenScope {
        key: ScopeKey::ApiKey,
        value: value.to_owned(),
        limit,
    }
}

fn window(
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

fn input(calls: Vec<BatchLimitCounterCall>) -> BatchLimitCountersInput {
    BatchLimitCountersInput {
        principal: BatchLimitPrincipal(BTreeMap::new()),
        calls,
    }
}

fn detail_text(value: &Payload) -> &str {
    match value {
        Payload::String(text) => text,
        other @ (Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Object(_)
        | Payload::Array(_)) => panic!("the detail is not text: {other:?}"),
    }
}

fn header(error: &ProxyRateLimitError, name: &str) -> String {
    let headers = error.headers.as_ref().expect("a rejection carries headers");
    let value = headers.get(name).expect("the named header is present");
    value.to_str().expect("the header is text").to_owned()
}

fn rejection(answer: &BatchLimitCounterAnswer) -> &ProxyRateLimitError {
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

// batch_rpm_limit: a requests counter over its limit rejects with a requests 429 and its reset window
#[rstest]
#[case::over_limit(6, BatchCounterCode::OverLimit, 2)]
#[case::at_limit(5, BatchCounterCode::Ok, 0)]
#[tokio::test]
async fn batch_rpm_limit(
    #[case] requests: i64,
    #[case] code: BatchCounterCode,
    #[case] remaining: i64,
) {
    let limits = descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None);
    let limiter = ScriptedLimiter::with_responses(vec![Ok(response(
        code,
        vec![status(
            code,
            "api_key",
            Some("sk-1"),
            BatchCounterLimitType::Requests,
            5,
            remaining,
        )],
        Vec::new(),
    ))]);
    let harness = harness(limiter, ScriptedStore::default(), ScriptedCache::default());
    let result = harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(Some("gpt-4o-mini")),
                batch_usage: usage(120, requests, 0),
                descriptors: Some(vec![limits.clone()]),
            },
        ]))
        .await
        .unwrap();
    let calls = harness.limiter.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, vec![limits]);
    assert_eq!(
        calls[0].1,
        vec![BatchDescriptorIncrements {
            requests,
            tokens: 120,
        }]
    );
    assert_eq!(result.answers.len(), 1);
    if code == BatchCounterCode::OverLimit {
        let error = rejection(&result.answers[0]);
        assert_eq!(error.category, RateLimitCategory::LitellmBatchRateLimit);
        assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
        assert_eq!(error.model, "gpt-4o-mini");
        assert_eq!(error.llm_provider, "openai");
        assert_eq!(
            detail_text(&error.detail),
            "Batch rate limit exceeded for api_key: sk-1. Batch contains 6 requests but only 2 requests remaining out of 5 RPM limit. Limit resets at: 2023-11-14 22:14:20 UTC"
        );
        assert_eq!(header(error, "retry-after"), "60");
        assert_eq!(header(error, "rate_limit_type"), "requests");
        assert_eq!(header(error, "reset_at"), RESET_AT);
        assert!(harness.stash.operations().is_empty());
    } else {
        match &result.answers[0] {
            BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection } => {
                assert!(rejection.is_none());
            }
            other => panic!("unexpected answer: {other:?}"),
        }
        assert!(harness.stash.operations().is_empty());
    }

    // no descriptors at all: the synthetic key is empty, the window falls back to the limiter's, and no model resolves
    let bare = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let empty = bare
        .service
        .run(&input(vec![BatchLimitCounterCall::RaiseRateLimitError {
            status: status(
                BatchCounterCode::OverLimit,
                "api_key",
                Some("sk-1"),
                BatchCounterLimitType::Requests,
                5,
                0,
            ),
            descriptors: Vec::new(),
            batch_usage: usage(120, 1, 0),
            limit_type: String::from("requests"),
            requested_model: None,
            window_start: None,
        }]))
        .await
        .unwrap();
    let empty_error = rejection(&empty.answers[0]);
    assert_eq!(empty_error.model, "");
    assert_eq!(empty_error.llm_provider, "litellm_proxy");
    assert_eq!(
        detail_text(&empty_error.detail),
        "Batch rate limit exceeded for : . Batch contains 1 requests but only 0 requests remaining out of 5 RPM limit. Limit resets at: 2023-11-14 22:14:20 UTC"
    );
    assert_eq!(header(empty_error, "retry-after"), "60");

    // a status that matches no descriptor reports the first descriptor's key and window
    let unmatched = bare
        .service
        .run(&input(vec![BatchLimitCounterCall::RaiseRateLimitError {
            status: status(
                BatchCounterCode::OverLimit,
                "api_key_tpd",
                Some("sk-1"),
                BatchCounterLimitType::Tokens,
                100,
                0,
            ),
            descriptors: vec![descriptor(
                "team",
                "team-1",
                Some(TPD_WINDOW_SECONDS),
                None,
                Some(100),
            )],
            batch_usage: usage(120, 1, 0),
            limit_type: String::from("tokens"),
            requested_model: Some(String::from("gpt-4o-mini")),
            window_start: Some(1_699_990_000),
        }]))
        .await
        .unwrap();
    let unmatched_error = rejection(&unmatched.answers[0]);
    assert_eq!(
        detail_text(&unmatched_error.detail),
        "Batch rate limit exceeded for team: team-1. Batch contains 120 tokens but only 0 tokens remaining out of 100 TPD limit. Limit resets at: 2023-11-15 19:26:40 UTC"
    );
    assert_eq!(header(unmatched_error, "retry-after"), "76400");
}

// batch_tpm_limit: the token figure is the charged model's row for project quotas, the batch total otherwise
#[rstest]
#[case::batch_total("api_key", "sk-1", None, None, None, "TPM", 120)]
#[case::tpd_label("api_key_tpd", "sk-1", None, None, None, "TPD", 120)]
#[case::project_otpm_row(
    "model_per_project_otpm",
    "proj:gpt-4o",
    Some("gpt-4o"),
    None,
    Some(30),
    "TPM",
    30
)]
#[case::project_otpm_absent_falls_back(
    "model_per_project_otpm",
    "proj:gpt-4o",
    Some("gpt-4o"),
    Some(40),
    None,
    "TPM",
    7
)]
#[case::project_itpm_row(
    "model_per_project_itpm",
    "proj:gpt-4o",
    Some("gpt-4o"),
    Some(40),
    None,
    "TPM",
    40
)]
#[case::project_itpm_absent_falls_back(
    "model_per_project_itpm",
    "proj:gpt-4o",
    Some("gpt-4o"),
    None,
    Some(30),
    "TPM",
    120
)]
#[case::project_value_without_colon(
    "model_per_project_itpm",
    "gpt-4o",
    Some("gpt-4o"),
    Some(40),
    None,
    "TPM",
    40
)]
#[tokio::test]
async fn batch_tpm_limit(
    #[case] key: &str,
    #[case] value: &str,
    #[case] model: Option<&str>,
    #[case] row_total: Option<i64>,
    #[case] row_output: Option<i64>,
    #[case] label: &str,
    #[case] tokens: i64,
) {
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
    let limits = descriptor(key, value, Some(WINDOW), None, Some(100));
    let result = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    )
    .service
    .run(&input(vec![BatchLimitCounterCall::RaiseRateLimitError {
        status: status(
            BatchCounterCode::OverLimit,
            key,
            Some(value),
            BatchCounterLimitType::Tokens,
            100,
            5,
        ),
        descriptors: vec![limits],
        batch_usage,
        limit_type: String::from("tokens"),
        requested_model: Some(String::from("gpt-4o-mini")),
        window_start: None,
    }]))
    .await
    .unwrap();
    let error = rejection(&result.answers[0]);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    let expected = format!(
        "Batch rate limit exceeded for {key}: {value}. Batch contains {tokens} tokens but only 5 tokens remaining out of 100 {label} limit. Limit resets at: {RESET_AT}"
    );
    assert_eq!(detail_text(&error.detail), expected);
    assert_eq!(header(error, "retry-after"), "60");
    assert_eq!(header(error, "rate_limit_type"), "tokens");
    assert_eq!(header(error, "reset_at"), RESET_AT);
}

#[derive(Clone, Copy)]
enum StoreReplyKind {
    Reservation,
    OverLimit,
}

// batch_enqueued_limit: a grant is stashed, an over-limit scope rejects, and data's model stays optional
#[rstest]
#[case::reserved(StoreReplyKind::Reservation, None)]
#[case::over_limit(StoreReplyKind::OverLimit, Some("gpt-4o-mini"))]
#[case::over_limit_without_model(StoreReplyKind::OverLimit, None)]
#[tokio::test]
async fn batch_enqueued_limit(#[case] reply: StoreReplyKind, #[case] model: Option<&str>) {
    let scopes = vec![scope("sk-1", 15)];
    let store_reply = match reply {
        StoreReplyKind::Reservation => reserved(30, &scopes),
        StoreReplyKind::OverLimit => over_limit(scope("sk-1", 15), 10),
    };
    let harness = harness(
        ScriptedLimiter::default(),
        ScriptedStore::with_replies(vec![Ok(store_reply)]),
        ScriptedCache::default(),
    );
    let result = harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data: data(model),
                batch_usage: usage(30, 2, 0),
                scopes: scopes.clone(),
            },
        ]))
        .await
        .unwrap();
    assert_eq!(harness.store.calls(), vec![(30, scopes.clone())]);
    match reply {
        StoreReplyKind::Reservation => {
            let reservation = harness.stash.reservation().expect("the grant is stashed");
            assert_eq!(reservation.tokens, 30);
            assert_eq!(reservation.scopes, scopes);
            assert_eq!(reservation.backend, ReservationBackend::Memory);
            match &result.answers[0] {
                BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection } => {
                    assert!(rejection.is_none());
                }
                other => panic!("unexpected answer: {other:?}"),
            }
        }
        StoreReplyKind::OverLimit => {
            let error = rejection(&result.answers[0]);
            assert_eq!(error.category, RateLimitCategory::LitellmBatchRateLimit);
            assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
            assert_eq!(
                detail_text(&error.detail),
                "Batch enqueued token limit exceeded for api_key: sk-1. Batch requires 30 tokens but only 5 enqueued tokens remaining out of 15 enqueued token limit. Tokens free up as running batches complete or are cancelled."
            );
            assert_eq!(header(error, "rate_limit_type"), "tokens");
            match model {
                Some(name) => {
                    assert_eq!(error.model, name);
                    assert_eq!(error.llm_provider, "openai");
                }
                None => {
                    assert_eq!(error.model, "");
                    assert_eq!(error.llm_provider, "litellm_proxy");
                }
            }
            assert!(harness.stash.reservation().is_none());
        }
    }

    // the direct callable reads data's model too, and an explicit null is not a model
    for (fields, expected_model, expected_provider) in [
        (data(Some("gpt-4o-mini")), "gpt-4o-mini", "openai"),
        (null_model_data(), "", "litellm_proxy"),
        (
            data(Some("unknown-batch-model")),
            "unknown-batch-model",
            "litellm_proxy",
        ),
    ] {
        let direct = harness
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
        let direct_error = rejection(&direct.answers[0]);
        assert_eq!(direct_error.model, expected_model);
        assert_eq!(direct_error.llm_provider, expected_provider);
    }
}

#[derive(Clone, Copy)]
enum Stored {
    Integer(i64),
    Float(f64),
    Text(&'static str),
}

fn stored_value(stored: Stored) -> WindowStartValue {
    match stored {
        Stored::Integer(value) => WindowStartValue::Integer(value),
        Stored::Float(value) => WindowStartValue::Float(value),
        Stored::Text(text) => WindowStartValue::Text(text.to_owned()),
    }
}

// tpd_window_refund: a daily window's stored value truncates, and its refund op carries the window identity
#[rstest]
#[case::integer(Some(Stored::Integer(1_699_990_000)), Some(1_699_990_000))]
#[case::float_truncates(Some(Stored::Float(1_699_990_000.9)), Some(1_699_990_000))]
#[case::numeric_text(Some(Stored::Text(" 1699990000.5 ")), Some(1_699_990_000))]
#[case::missing(None, None)]
#[case::invalid_text(Some(Stored::Text("yesterday")), None)]
#[tokio::test]
async fn tpd_window_refund(#[case] stored: Option<Stored>, #[case] expected_start: Option<i64>) {
    let cache = match stored {
        Some(value) => ScriptedCache::with_value("{api_key_tpd:sk-1}:window", stored_value(value)),
        None => ScriptedCache::default(),
    };
    let harness = harness(ScriptedLimiter::default(), ScriptedStore::default(), cache);
    let window = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::ReadTpdWindowStart {
            status: status(
                BatchCounterCode::OverLimit,
                "api_key_tpd",
                Some("sk-1"),
                BatchCounterLimitType::Tokens,
                10,
                0,
            ),
        }]))
        .await
        .unwrap();
    match &window.answers[0] {
        BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => {
            assert_eq!(*window_start, expected_start);
        }
        other => panic!("unexpected answer: {other:?}"),
    }
    assert_eq!(
        harness.cache.reads(),
        vec![String::from("{api_key_tpd:sk-1}:window")]
    );

    // a per-minute counter has no window key, so the cache is never read
    let plain = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let answer = plain
        .service
        .run(&input(vec![BatchLimitCounterCall::ReadTpdWindowStart {
            status: status(
                BatchCounterCode::OverLimit,
                "api_key",
                Some("sk-1"),
                BatchCounterLimitType::Tokens,
                10,
                0,
            ),
        }]))
        .await
        .unwrap();
    match &answer.answers[0] {
        BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => {
            assert!(window_start.is_none());
        }
        other => panic!("unexpected answer: {other:?}"),
    }
    assert!(plain.cache.reads().is_empty());

    // an absent descriptor value builds the window key with an empty value
    let valueless = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::with_value(
            "{team_tpd:}:window",
            WindowStartValue::Integer(1_699_990_000),
        ),
    );
    let found = valueless
        .service
        .run(&input(vec![BatchLimitCounterCall::ReadTpdWindowStart {
            status: status(
                BatchCounterCode::OverLimit,
                "team_tpd",
                None,
                BatchCounterLimitType::Tokens,
                10,
                0,
            ),
        }]))
        .await
        .unwrap();
    match &found.answers[0] {
        BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => {
            assert_eq!(*window_start, Some(1_699_990_000));
        }
        other => panic!("unexpected answer: {other:?}"),
    }
    assert_eq!(
        valueless.cache.reads(),
        vec![String::from("{team_tpd:}:window")]
    );
}

// tpd_window_refund: the daily counters this batch charged become negative-token refund ops
#[rstest]
#[case::positive(120, 3)]
#[case::zero(0, 0)]
#[case::negative(-5, 0)]
#[tokio::test]
async fn tpd_window_refund_operations(#[case] tokens: i64, #[case] expected: usize) {
    let daily = descriptor(
        "api_key_tpd",
        "sk-1",
        Some(TPD_WINDOW_SECONDS),
        None,
        Some(100),
    );
    let minute = descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None);
    let windows = BTreeSet::from([
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
    ]);
    let harness = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let result = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::BuildTpdRefundOps {
            descriptors: vec![daily, minute],
            tokens,
            reservation_windows: windows,
        }]))
        .await
        .unwrap();
    match &result.answers[0] {
        BatchLimitCounterAnswer::BuildTpdRefundOps { operations } => {
            assert_eq!(operations.len(), expected);
            if expected > 0 {
                let expected_ops = vec![
                    BatchTpdRefundOperation {
                        key: String::from("{api_key_tpd:sk-1}:tokens"),
                        increment_value: -tokens,
                        ttl: TPD_WINDOW_SECONDS,
                        window_key: String::from("{api_key_tpd:sk-1}:window"),
                        expected_window_start: String::from("1699990000"),
                        reservation_backend: BatchReservationBackend::Local,
                    },
                    BatchTpdRefundOperation {
                        key: String::from("{api_key_tpd:sk-1}:tokens"),
                        increment_value: -tokens,
                        ttl: TPD_WINDOW_SECONDS,
                        window_key: String::from("{api_key_tpd:sk-1}:window"),
                        expected_window_start: String::from("1699990000"),
                        reservation_backend: BatchReservationBackend::Redis,
                    },
                    BatchTpdRefundOperation {
                        key: String::from("{api_key_tpd:sk-1}:tokens"),
                        increment_value: -tokens,
                        ttl: TPD_WINDOW_SECONDS,
                        window_key: String::from("{api_key_tpd:sk-1}:window"),
                        expected_window_start: String::from("1699990001"),
                        reservation_backend: BatchReservationBackend::Local,
                    },
                ];
                assert_eq!(*operations, expected_ops);
            }
        }
        other => panic!("unexpected answer: {other:?}"),
    }

    // a check that succeeds stashes the refunds, and a window-aware rejection resets from that window
    let daily = descriptor(
        "api_key_tpd",
        "sk-1",
        Some(TPD_WINDOW_SECONDS),
        None,
        Some(100),
    );
    let limiter = ScriptedLimiter::with_responses(vec![Ok(ok_response(
        Vec::new(),
        vec![(
            "{api_key_tpd:sk-1}:tokens",
            "1699990000",
            BatchReservationBackend::Redis,
        )],
    ))]);
    let harness = harness(limiter, ScriptedStore::default(), ScriptedCache::default());
    harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(Some("gpt-4o-mini")),
                batch_usage: usage(120, 3, 0),
                descriptors: Some(vec![daily.clone()]),
            },
        ]))
        .await
        .unwrap();
    let stashed = harness.stash.operations();
    assert_eq!(stashed.len(), 1);
    assert_eq!(stashed[0].key, "{api_key_tpd:sk-1}:tokens");
    assert_eq!(stashed[0].increment_value, -120);
    let aware = harness
        .service
        .run(&input(vec![BatchLimitCounterCall::RaiseRateLimitError {
            status: status(
                BatchCounterCode::OverLimit,
                "api_key_tpd",
                Some("sk-1"),
                BatchCounterLimitType::Tokens,
                100,
                5,
            ),
            descriptors: vec![daily],
            batch_usage: usage(120, 3, 0),
            limit_type: String::from("tokens"),
            requested_model: Some(String::from("gpt-4o-mini")),
            window_start: Some(1_699_990_000),
        }]))
        .await
        .unwrap();
    let error = rejection(&aware.answers[0]);
    assert_eq!(header(error, "retry-after"), "76400");
    assert_eq!(header(error, "reset_at"), "2023-11-15 19:26:40 UTC");
}

// counter_partial_failure: the first over-limit counter rejects, and a later failure leaves the stash's earlier refunds
#[rstest]
#[case::first_over_limit_wins(false)]
#[case::limiter_failure_keeps_stash(true)]
#[tokio::test]
async fn counter_partial_failure(#[case] limiter_fails: bool) {
    if limiter_fails {
        // the first call stashes refunds; the second fails at the limiter, which must not clear them
        let daily = descriptor(
            "api_key_tpd",
            "sk-1",
            Some(TPD_WINDOW_SECONDS),
            None,
            Some(100),
        );
        let ok = ScriptedLimiter::with_responses(vec![Ok(ok_response(
            Vec::new(),
            vec![(
                "{api_key_tpd:sk-1}:tokens",
                "1699990000",
                BatchReservationBackend::Redis,
            )],
        ))]);
        let harness = harness(ok, ScriptedStore::default(), ScriptedCache::default());
        harness
            .service
            .run(&input(vec![
                BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                    data: data(Some("gpt-4o-mini")),
                    batch_usage: usage(120, 3, 0),
                    descriptors: Some(vec![daily]),
                },
            ]))
            .await
            .unwrap();
        assert_eq!(harness.stash.operations().len(), 1);
        let failure = harness
            .service
            .run(&input(vec![
                BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                    data: data(Some("gpt-4o-mini")),
                    batch_usage: usage(10, 1, 0),
                    descriptors: Some(vec![descriptor(
                        "team",
                        "team-1",
                        Some(WINDOW),
                        Some(1),
                        None,
                    )]),
                },
            ]))
            .await;
        assert!(matches!(failure, Err(BatchLimitCountersError::Limits(_))));
        assert_eq!(harness.stash.operations().len(), 1);
        return;
    }
    let base = vec![
        descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None),
        descriptor(
            "api_key_tpd",
            "sk-1",
            Some(TPD_WINDOW_SECONDS),
            None,
            Some(100),
        ),
    ];
    let project = descriptor(
        "model_per_project_otpm",
        "proj:gpt-4o",
        Some(WINDOW),
        None,
        Some(100),
    );
    let limiter = ScriptedLimiter {
        base,
        project: (
            vec![project.clone()],
            vec![BatchDescriptorIncrements {
                requests: 0,
                tokens: 30,
            }],
        ),
        ..ScriptedLimiter::with_responses(vec![Ok(response(
            BatchCounterCode::OverLimit,
            vec![
                status(
                    BatchCounterCode::Ok,
                    "api_key",
                    Some("sk-1"),
                    BatchCounterLimitType::Requests,
                    5,
                    1,
                ),
                status(
                    BatchCounterCode::OverLimit,
                    "api_key_tpd",
                    Some("sk-1"),
                    BatchCounterLimitType::Tokens,
                    100,
                    0,
                ),
                status(
                    BatchCounterCode::OverLimit,
                    "team_tpd",
                    Some("team-1"),
                    BatchCounterLimitType::Tokens,
                    100,
                    0,
                ),
            ],
            Vec::new(),
        ))])
    };
    let cache = ScriptedCache {
        values: BTreeMap::from([
            (
                String::from("{api_key_tpd:sk-1}:window"),
                WindowStartValue::Integer(1_699_990_000),
            ),
            (
                String::from("{team_tpd:team-1}:window"),
                WindowStartValue::Integer(1_699_980_000),
            ),
        ]),
        ..ScriptedCache::default()
    };
    let harness = harness(limiter, ScriptedStore::default(), cache);
    let mut per_model = IndexMap::new();
    per_model.insert(
        String::from("gpt-4o"),
        BTreeMap::from([(String::from("output_tokens"), 30)]),
    );
    let mut batch_usage = usage(120, 3, 30);
    batch_usage.per_model_usage = per_model;
    let result = harness
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(Some("gpt-4o-mini")),
                batch_usage,
                descriptors: None,
            },
        ]))
        .await
        .unwrap();
    let calls = harness.limiter.calls();
    assert_eq!(harness.limiter.base_calls(), 1);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.len(), 3);
    assert_eq!(calls[0].0[0].key, "api_key");
    assert_eq!(calls[0].0[1].key, "api_key_tpd");
    assert_eq!(calls[0].0[2], project);
    assert_eq!(
        calls[0].1,
        vec![
            BatchDescriptorIncrements {
                requests: 3,
                tokens: 120,
            },
            BatchDescriptorIncrements {
                requests: 3,
                tokens: 120,
            },
            BatchDescriptorIncrements {
                requests: 0,
                tokens: 30,
            },
        ]
    );
    let error = rejection(&result.answers[0]);
    assert_eq!(
        detail_text(&error.detail),
        "Batch rate limit exceeded for api_key_tpd: sk-1. Batch contains 120 tokens but only 0 tokens remaining out of 100 TPD limit. Limit resets at: 2023-11-15 19:26:40 UTC"
    );
    assert_eq!(header(error, "retry-after"), "76400");
    // only the first over-limit status reads its window
    assert_eq!(
        harness.cache.reads(),
        vec![String::from("{api_key_tpd:sk-1}:window")]
    );
    assert!(harness.stash.operations().is_empty());
}
