#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::rate_limiter_settings;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitDimension,
};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use betterlite_rate_limit::identity_descriptors::agent_registry::AgentPolicyRegistry;
use betterlite_rate_limit::limiter_assembly::{CallTypeRateLimiter, RateLimiterAssembly};
use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, LimiterClock, ModelGroupResolver, TagRateLimit, TagRateLimitResolver,
};
use betterlite_rate_limit::pre_call_hook::{PreCallHookError, RateLimitPreCallHook};
use betterlite_rate_limit::rate_limit_window_store::{
    CounterCache, CounterValues, RedisCacheKind, WindowStore,
};
use betterlite_rate_limit::request_scopes::deployment_health::{
    DeploymentHealthError, DeploymentHealthSource, DeploymentRef,
};
use betterlite_rate_limit::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use betterlite_rate_limit::token_estimation::output_cap::CapEndpoint;
use betterlite_rate_limit::token_estimation::services::{
    GenAiTranslation, GenAiTranslationRequest, InputTokenCounter, ResponsesInput,
    ResponsesInputRequest, TokenCounterRequest, TranslatedChat,
};
use betterlite_rate_limit::windowed_admission::{
    SlotIdGenerator, WindowedAdmission, WindowedAdmissionError,
};
use betterlite_redact::structured::Payload;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;

use super::{
    LimiterSelection, RateLimitGateRequest, RateLimitPreCallError, RateLimitPreCallGate,
    RequestRateLimitState,
};

struct Noop;

fn test_auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).expect("the auth fixture builds")
}

#[async_trait::async_trait]
impl CounterCache for Noop {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

#[async_trait::async_trait]
impl GaugeCache for Noop {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

#[async_trait::async_trait]
impl UsageIncrements for Noop {
    async fn increment(&self, _operation: &IncrementOperation) {}

    async fn increment_pipeline(&self, _operations: &[IncrementOperation]) {}

    async fn set_local_counter(&self, _key: &str, _value: f64, _ttl: Option<i64>) {}
}

#[async_trait::async_trait]
impl DeploymentHealthSource for Noop {
    async fn deployments_for_model(
        &self,
        _model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        Ok(None)
    }

    async fn failures_this_minute(
        &self,
        _deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        Ok(0)
    }
}

impl AgentPolicyRegistry for Noop {
    fn agent_by_id(&self, _agent_id: &str) -> Option<AgentResponse> {
        None
    }
}

impl LimiterClock for Noop {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .expect("the fixed clock is after the epoch")
    }
}

impl SlotIdGenerator for Noop {
    fn next_slot_id(&self) -> String {
        String::from("slot-1")
    }
}

impl GenAiTranslation for Noop {
    fn translate_generate_content(
        &self,
        _request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        None
    }
}

impl ResponsesInput for Noop {
    fn transform_responses_api_input(&self, _request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        Vec::new()
    }
}

#[async_trait::async_trait]
impl InputTokenCounter for Noop {
    async fn count_tokens(&self, _request: &TokenCounterRequest<'_>) -> Option<i64> {
        None
    }
}

struct NoGroups;

impl ModelGroupResolver for NoGroups {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

struct NoTags;

#[async_trait::async_trait]
impl TagRateLimitResolver for NoTags {
    async fn resolve(&self, _tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        BTreeMap::new()
    }
}

struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

fn window_store() -> WindowStore {
    WindowStore {
        cache: Arc::new(Noop),
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: Mutex::new(()),
    }
}

fn test_gauges(clock: &Arc<dyn LimiterClock>, fail_closed: FailClosedPolicy) -> ConcurrencyGauges {
    ConcurrencyGauges {
        cache: Arc::new(Noop),
        clock: Arc::clone(clock),
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed,
        check_and_increment_lock: Mutex::new(()),
    }
}

fn test_admission(
    clock: &Arc<dyn LimiterClock>,
    fail_closed: FailClosedPolicy,
    windows: &Arc<WindowStore>,
    gauges: &Arc<ConcurrencyGauges>,
) -> WindowedAdmission {
    WindowedAdmission {
        window_size: 60,
        clock: Arc::clone(clock),
        fail_closed,
        windows: Arc::clone(windows),
        refund_script: None,
        gauges: Arc::clone(gauges),
        slot_ids: Arc::new(Noop),
    }
}

fn test_adjustments() -> TokenAdjustments {
    TokenAdjustments {
        windows: window_store(),
        usage: Arc::new(Noop),
        post_call_batch: None,
        token_increment_script: None,
        guarded_token_script: None,
    }
}

fn assembly(batch: Option<Arc<dyn CallTypeRateLimiter>>) -> Arc<RateLimiterAssembly> {
    let settings = rate_limiter_settings::Settings {
        fail_closed: false,
        window_size: 60,
        tpm_reservation_enabled: true,
    };
    let clock: Arc<dyn LimiterClock> = Arc::new(Noop);
    let fail_closed = FailClosedPolicy::default();
    let windows = Arc::new(window_store());
    let gauges = Arc::new(test_gauges(&clock, fail_closed));
    let admission = Arc::new(test_admission(&clock, fail_closed, &windows, &gauges));
    Arc::new(RateLimiterAssembly {
        settings,
        clock,
        fail_closed,
        windows,
        check_and_increment_script: None,
        gauges,
        admission,
        adjustments: Arc::new(test_adjustments()),
        model_groups: Box::new(NoGroups),
        batch_limiter: StdMutex::new(batch),
    })
}

fn hooks(batch: Option<Arc<dyn CallTypeRateLimiter>>) -> RateLimitPreCallHook {
    RateLimitPreCallHook {
        assembly: assembly(batch),
        model_groups: Arc::new(NoGroups),
        agents: Arc::new(Noop),
        health: Arc::new(Noop),
        error_threshold_per_minute: 1,
        tags: Arc::new(NoTags),
        translation: Arc::new(Noop),
        responses: Arc::new(Noop),
        input_tokens: Arc::new(Noop),
        native: Arc::new(NoProviders),
        router: None,
        tasks: Arc::new(TaskTracker::new()),
    }
}

async fn run(
    gate: &RateLimitPreCallGate,
    auth: &UserAPIKeyAuth,
    data: &mut RequestData,
    state: &RequestRateLimitState,
    call_type: CallType,
) -> Result<(), RateLimitPreCallError> {
    gate.pre_call(RateLimitGateRequest {
        auth,
        data,
        call_type,
        endpoint: CapEndpoint::Generic,
        state,
    })
    .await
}

async fn body_of<Body>(response: Response) -> Body
where
    Body: DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the error body reads");
    serde_json::from_slice(&bytes).expect("the error body is JSON")
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct DetailBody<Detail> {
    detail: Detail,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ErrorDetail {
    error: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ProxyErrorEnvelope {
    error: ProxyErrorFields,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ProxyErrorFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

#[tokio::test]
async fn main_limiter_returns_the_v3_hook() {
    let gate = RateLimitPreCallGate {
        selection: LimiterSelection::V3(hooks(None)),
    };

    assert!(gate.main_limiter().is_some());
}

#[tokio::test]
async fn main_limiter_returns_none_for_the_legacy_limiter() {
    let gate = RateLimitPreCallGate {
        selection: LimiterSelection::Legacy,
    };

    assert!(gate.main_limiter().is_none());
}

#[tokio::test]
async fn legacy_pre_call_passes_without_touching_the_stash() {
    let gate = RateLimitPreCallGate {
        selection: LimiterSelection::Legacy,
    };
    let state = RequestRateLimitState::default();

    run(
        &gate,
        &test_auth(),
        &mut RequestData::default(),
        &state,
        CallType::Acompletion,
    )
    .await
    .expect("the legacy limiter passes every request");

    assert!(state.stash.get().is_none());
}

#[tokio::test]
async fn batch_call_without_a_batch_limiter_maps_to_503() {
    let gate = RateLimitPreCallGate {
        selection: LimiterSelection::V3(hooks(None)),
    };
    let state = RequestRateLimitState::default();

    let error = run(
        &gate,
        &test_auth(),
        &mut RequestData::default(),
        &state,
        CallType::AcreateBatch,
    )
    .await
    .expect_err("the batch call needs a batch limiter");

    assert!(matches!(
        error,
        RateLimitPreCallError::MissingBatchLimiter {
            call_type: CallType::AcreateBatch
        }
    ));
    let message = error.to_string();
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_of::<DetailBody<ErrorDetail>>(response).await,
        DetailBody {
            detail: ErrorDetail { error: message },
        }
    );
}

#[tokio::test]
async fn rate_limited_hook_error_maps_to_429_with_its_headers() {
    let error = ProxyRateLimitError::new(ProxyRateLimitErrorInit {
        detail: Payload::String(String::from("Rate limit exceeded for api_key: key-1")),
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            Payload::String(String::from("0")),
        )])),
        category: None,
        rate_limit_type: Some(RateLimitDimension::Requests),
        model: None,
        llm_provider: None,
    })
    .expect("the rate limit error builds");

    let response =
        RateLimitPreCallError::from(PreCallHookError::RateLimited(Box::new(error))).into_response();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(header::RETRY_AFTER),
        Some(&HeaderValue::from_static("0"))
    );
    assert_eq!(
        body_of::<DetailBody<String>>(response).await,
        DetailBody {
            detail: String::from("Rate limit exceeded for api_key: key-1"),
        }
    );
}

#[tokio::test]
async fn unverifiable_hook_error_maps_to_503() {
    let error = PreCallHookError::Admission(WindowedAdmissionError::Unverifiable(
        RateLimitUnverifiableError {
            status_code: 503,
            detail: BTreeMap::from([(String::from("error"), String::from("counters unavailable"))]),
        },
    ));

    let response = RateLimitPreCallError::from(error).into_response();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_of::<DetailBody<ErrorDetail>>(response).await,
        DetailBody {
            detail: ErrorDetail {
                error: String::from("counters unavailable"),
            },
        }
    );
}

#[tokio::test]
async fn unverifiable_hook_error_with_an_unusable_status_maps_to_500() {
    let error = PreCallHookError::Admission(WindowedAdmissionError::Unverifiable(
        RateLimitUnverifiableError {
            status_code: 7000,
            detail: BTreeMap::from([(String::from("error"), String::from("counters unavailable"))]),
        },
    ));
    let cause = StatusCode::from_u16(7000).expect_err("7000 is not an HTTP status");

    let response = RateLimitPreCallError::from(error).into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_of::<ProxyErrorEnvelope>(response).await,
        ProxyErrorEnvelope {
            error: ProxyErrorFields {
                message: format!(
                    "the fail-closed rate-limit rejection carries status 7000 that HTTP cannot carry: {cause}"
                ),
                error_type: String::from("internal_server_error"),
                param: None,
                code: String::from("500"),
            },
        }
    );
}

#[tokio::test]
async fn other_hook_errors_map_to_500() {
    let message = PreCallHookError::AdmissionTaskLost.to_string();
    let error = PreCallHookError::AdmissionTaskLost;

    let response = RateLimitPreCallError::from(error).into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_of::<ProxyErrorEnvelope>(response).await,
        ProxyErrorEnvelope {
            error: ProxyErrorFields {
                message,
                error_type: String::from("internal_server_error"),
                param: None,
                code: String::from("500"),
            },
        }
    );
}

#[tokio::test]
async fn every_call_of_one_request_shares_the_stash() {
    let gate = RateLimitPreCallGate {
        selection: LimiterSelection::V3(hooks(None)),
    };
    let auth = test_auth();
    let mut data = RequestData::default();
    let state = RequestRateLimitState::default();

    run(&gate, &auth, &mut data, &state, CallType::Acompletion)
        .await
        .expect("the first call passes");
    let first = state.stash.get().expect("the first call claimed the stash");
    run(&gate, &auth, &mut data, &state, CallType::Acompletion)
        .await
        .expect("the second call passes");
    let second = state
        .stash
        .get()
        .expect("the stash stays for the whole request");

    assert!(Arc::ptr_eq(&first, &second));
}
