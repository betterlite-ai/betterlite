use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_rate_limit::atomic_counter::payload::IncrementAmounts;
use betterlite_rate_limit::dynamic_admission::admission::{
    AdmissionKey, DynamicAdmission, RateLimitCheck, RateLimitProviderResolver,
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
use betterlite_rate_limit::settlement_and_rollback::SettlementError;
use serde::Deserialize;
use tokio::runtime::Builder;

mod output;

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

// one call the handler made, kept in order so the cases can check every descriptor and increment it sent
enum RecordedCall {
    Atomic(AtomicCall),
    Tracking(TrackingCall),
}

// the v3 limiter the hook calls behind the port, answering the canned responses and recording each call
struct StubLimiter {
    window_size: i64,
    atomic: RateLimitResponse,
    tracking: RateLimitResponse,
    calls: Mutex<Vec<RecordedCall>>,
}

impl StubLimiter {
    fn new(input: &Input) -> Self {
        Self {
            window_size: input.window_size,
            atomic: input.atomic.clone(),
            tracking: input.tracking.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: RecordedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
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
        if increments.len() != descriptors.len() {
            return Err(AdmissionLimiterError::from_settlement(
                SettlementError::DescriptorCount {
                    descriptors: descriptors.len(),
                    increments: increments.len(),
                },
            ));
        }
        self.record(RecordedCall::Atomic(AtomicCall {
            descriptors: descriptors.to_vec(),
            increments: increments.to_vec(),
        }));
        Ok(self.atomic.clone())
    }

    async fn should_rate_limit(
        &self,
        descriptors: &[RateLimitDescriptor],
        _parent_span: Option<ParentOtelSpan>,
        read_only: bool,
    ) -> Result<RateLimitResponse, AdmissionLimiterError> {
        self.record(RecordedCall::Tracking(TrackingCall {
            descriptors: descriptors.to_vec(),
            read_only,
        }));
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
        output::answer(outcome, &limiter, &stash)
    }
}
