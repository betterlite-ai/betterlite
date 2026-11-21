use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_rate_limit::atomic_counter::payload::IncrementAmounts;
use betterlite_rate_limit::dynamic_admission::admission::{
    AdmissionKey, CheckRateLimitError, DynamicAdmission, RateLimitCheck, RateLimitProviderResolver,
};
use betterlite_rate_limit::dynamic_admission::limiter_port::{
    AdmissionLimiter, AdmissionLimiterError,
};
use betterlite_rate_limit::dynamic_priority::cache::SaturationCache;
use betterlite_rate_limit::dynamic_priority::handler::DynamicRateLimitHandler;
use betterlite_rate_limit::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, RateLimitDescriptor, RateLimitResponse,
};
use betterlite_rate_limit::provider_attribution::{ModelGroupLimits, PriorityValue};
use betterlite_rate_limit::request_stash::RequestStashSlot;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    model: String,
    rpm: Option<i64>,
    tpm: Option<i64>,
    window_size: i64,
    priority: Option<String>,
    priority_reservation: Option<BTreeMap<String, PriorityValue>>,
    default_priority: f64,
    saturation_threshold: f64,
    saturation: f64,
    atomic: RateLimitResponse,
    tracking: RateLimitResponse,
}

// the hook reads a model group's capacities from the router; _check_rate_limits itself never needs one
struct EmptyCache;

#[async_trait::async_trait]
impl SaturationCache for EmptyCache {
    async fn get_cache(
        &self,
        _key: &str,
        _local_only: bool,
        _local_ttl: i64,
    ) -> Option<CacheCounterValue> {
        None
    }
}

// resolve_llm_provider_for_rate_limit runs once per rejection; the cases compare the 429, not the pair
struct FixedProviders;

impl RateLimitProviderResolver for FixedProviders {
    fn resolve(&self, model: &str) -> (String, String) {
        (model.to_owned(), "openai".to_owned())
    }
}

#[derive(Clone)]
struct AtomicCall {
    descriptors: Vec<RateLimitDescriptor>,
    increments: Vec<IncrementAmounts>,
}

#[derive(Clone)]
struct TrackingCall {
    descriptors: Vec<RateLimitDescriptor>,
    read_only: bool,
}

#[derive(Default)]
struct RecordedCalls {
    atomic: Vec<AtomicCall>,
    tracking: Vec<TrackingCall>,
}

// the v3 limiter the hook calls behind the port, answering the canned responses and recording each call
struct StubLimiter {
    window_size: i64,
    atomic: RateLimitResponse,
    tracking: RateLimitResponse,
    calls: Mutex<RecordedCalls>,
}

impl StubLimiter {
    fn new(input: &Input) -> Self {
        Self {
            window_size: input.window_size,
            atomic: input.atomic.clone(),
            tracking: input.tracking.clone(),
            calls: Mutex::new(RecordedCalls::default()),
        }
    }

    fn recorded(&self) -> (Vec<AtomicCall>, Vec<TrackingCall>) {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        (calls.atomic.clone(), calls.tracking.clone())
    }
}

#[async_trait::async_trait]
impl AdmissionLimiter for StubLimiter {
    fn window_size(&self) -> i64 {
        self.window_size
    }

    // ast-grep-ignore: no-long-names
    async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
        _parent_span: Option<ParentOtelSpan>,
    ) -> Result<RateLimitResponse, AdmissionLimiterError> {
        let call = AtomicCall {
            descriptors: descriptors.to_vec(),
            increments: increments.to_vec(),
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.atomic.push(call);
        drop(calls);
        Ok(self.atomic.clone())
    }

    async fn should_rate_limit(
        &self,
        descriptors: &[RateLimitDescriptor],
        _parent_span: Option<ParentOtelSpan>,
        read_only: bool,
    ) -> Result<RateLimitResponse, AdmissionLimiterError> {
        let call = TrackingCall {
            descriptors: descriptors.to_vec(),
            read_only,
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.tracking.push(call);
        drop(calls);
        Ok(self.tracking.clone())
    }
}

fn policy_from(input: &Input) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings {
            default_priority: input.default_priority,
            saturation_threshold: input.saturation_threshold,
            saturation_check_cache_ttl: 60,
        },
        reservation: input.priority_reservation.clone(),
    }
}

fn descriptor_json(descriptors: &[RateLimitDescriptor]) -> Result<Vec<Value>, serde_json::Error> {
    descriptors.iter().map(serde_json::to_value).collect()
}

fn increments_json(increments: &[IncrementAmounts]) -> Vec<Value> {
    increments
        .iter()
        .map(|amount| json!({"requests": amount.requests, "tokens": amount.tokens}))
        .collect()
}

fn atomic_calls_json(calls: &[AtomicCall]) -> Result<Vec<Value>, serde_json::Error> {
    calls
        .iter()
        .map(|call| {
            Ok(json!({
                "descriptors": descriptor_json(&call.descriptors)?,
                "increments": increments_json(&call.increments),
            }))
        })
        .collect()
}

fn tracking_calls_json(calls: &[TrackingCall]) -> Result<Vec<Value>, serde_json::Error> {
    calls
        .iter()
        .map(|call| {
            Ok(json!({
                "descriptors": descriptor_json(&call.descriptors)?,
                "read_only": call.read_only,
            }))
        })
        .collect()
}

fn stash_json(stash: &RequestStashSlot) -> Value {
    let Some(stored) = stash.get() else {
        return Value::Null;
    };
    let response = {
        let state = stored.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.rate_limit_response.clone()
    };
    response.map_or(Value::Null, |response| {
        json!({
            "overall_code": response.overall_code,
            "statuses": response.statuses,
        })
    })
}

fn headers_json(headers: &CheckRateLimitError) -> BTreeMap<String, String> {
    match headers {
        CheckRateLimitError::Rejection(error) => error
            .headers
            .iter()
            .flat_map(|map| map.iter())
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect(),
        CheckRateLimitError::Denial(_) | CheckRateLimitError::Limiter(_) => BTreeMap::new(),
    }
}

fn unexpected(message: &str) -> HelperResult {
    Err(Box::new(HelperError {
        class: "UnexpectedFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    }))
}

fn answer(
    outcome: Result<(), CheckRateLimitError>,
    limiter: &StubLimiter,
    stash: &RequestStashSlot,
) -> HelperResult {
    let (atomic, tracking) = limiter.recorded();
    let error = match &outcome {
        Ok(()) => Value::Null,
        Err(error @ CheckRateLimitError::Rejection(rejection)) => json!({
            "status": 429_u16,
            "detail": rejection.detail.clone(),
            "headers": headers_json(error),
        }),
        Err(error @ (CheckRateLimitError::Denial(_) | CheckRateLimitError::Limiter(_))) => {
            return unexpected(&error.to_string());
        }
    };
    Ok(json!({
        "atomic_calls": atomic_calls_json(&atomic)?,
        "tracking_calls": tracking_calls_json(&tracking)?,
        "stash": stash_json(stash),
        "error": error,
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let handler = DynamicRateLimitHandler::<()>::new(
            Arc::new(EmptyCache),
            None,
            policy_from(input),
            input.window_size,
        );
        let limiter = Arc::new(StubLimiter::new(input));
        let admission = DynamicAdmission {
            limiter: Arc::clone(&limiter),
            handler,
            providers: Arc::new(FixedProviders),
        };
        let key_metadata = BTreeMap::new();
        let key = AdmissionKey {
            team_metadata: None,
            key_metadata: &key_metadata,
            parent_otel_span: None,
        };
        let stash = RequestStashSlot::default();
        let check = RateLimitCheck {
            model: &input.model,
            model_group_info: ModelGroupLimits {
                rpm: input.rpm,
                tpm: input.tpm,
            },
            key: &key,
            priority: input.priority.as_deref(),
            saturation: input.saturation,
            stash: &stash,
        };
        let runtime = Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(admission.check_rate_limits(&check));
        answer(outcome, &limiter, &stash)
    }
}
