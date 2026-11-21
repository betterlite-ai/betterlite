use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::atomic_counter::payload::IncrementAmounts;
use crate::dynamic_admission::admission::{
    AdmissionKey, DynamicAdmission, RateLimitProviderResolver,
};
use crate::dynamic_admission::hook::{ModelGroupInfoError, ModelGroupInfoSource};
use crate::dynamic_admission::limiter_port::{AdmissionLimiter, AdmissionLimiterError};
use crate::dynamic_priority::cache::SaturationCache;
use crate::dynamic_priority::descriptors::SATURATION_DESCRIPTOR_KEY;
use crate::dynamic_priority::handler::DynamicRateLimitHandler;
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use crate::fail_closed::RateLimitUnverifiableError;
use crate::limiter_records::{
    CacheCounterValue, RateLimitDescriptor, RateLimitResponse, RateLimitStatus, RateLimitType,
    create_rate_limit_keys,
};
use crate::provider_attribution::{ModelGroupLimits, PriorityValue};
use crate::request_stash::RequestStashSlot;
use crate::settlement_and_rollback::SettlementError;

pub(crate) const WINDOW: i64 = 60;

pub(crate) const MODEL: &str = "gpt-4";

pub(crate) const fn limits(rpm: Option<i64>, tpm: Option<i64>) -> ModelGroupLimits {
    ModelGroupLimits { rpm, tpm }
}

pub(crate) const fn percent(value: f64) -> PriorityValue {
    PriorityValue::Percent(value)
}

pub(crate) fn policy(
    saturation_threshold: f64,
    reservation: Option<&[(&str, PriorityValue)]>,
) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings {
            default_priority: 0.25,
            saturation_threshold,
            saturation_check_cache_ttl: 60,
        },
        reservation: reservation.map(reservation_map),
    }
}

fn reservation_map(entries: &[(&str, PriorityValue)]) -> BTreeMap<String, PriorityValue> {
    let mut reservations = BTreeMap::new();
    for (priority, value) in entries {
        reservations.insert((*priority).to_owned(), *value);
    }
    reservations
}

pub(crate) fn counter_key(model: &str, rate_limit_type: RateLimitType) -> String {
    create_rate_limit_keys(SATURATION_DESCRIPTOR_KEY, model, rate_limit_type)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheRead {
    pub(crate) key: String,
    pub(crate) local_only: bool,
    pub(crate) local_ttl: i64,
}

// the usage cache the saturation check reads: seeded values, and every read recorded with the flags it got
#[derive(Default)]
pub(crate) struct RecordingCache {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Mutex<Vec<CacheRead>>,
}

impl RecordingCache {
    pub(crate) fn seeded(values: &[(String, CacheCounterValue)]) -> Self {
        let mut seeded = BTreeMap::new();
        for (key, value) in values {
            seeded.insert(key.clone(), value.clone());
        }
        Self {
            values: seeded,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<CacheRead> {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.clone()
    }
}

#[async_trait::async_trait]
impl SaturationCache for RecordingCache {
    async fn get_cache(
        &self,
        key: &str,
        local_only: bool,
        local_ttl: i64,
    ) -> Option<CacheCounterValue> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.push(CacheRead {
            key: key.to_owned(),
            local_only,
            local_ttl,
        });
        drop(reads);
        self.values.get(key).cloned()
    }
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture<F>(log: F) -> String
where
    F: FnOnce(),
{
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, log);
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

pub(crate) type Canned = Result<RateLimitResponse, AdmissionLimiterError>;

#[derive(Clone, Debug)]
pub(crate) struct AtomicCall {
    pub(crate) descriptors: Vec<RateLimitDescriptor>,
    pub(crate) increments: Vec<IncrementAmounts>,
    pub(crate) parent_span: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TrackingCall {
    pub(crate) descriptors: Vec<RateLimitDescriptor>,
    pub(crate) read_only: bool,
    pub(crate) parent_span: bool,
}

// the fake behind the port: it records each call and answers it with the case's canned result, once
pub(crate) struct RecordingLimiter {
    window_size: i64,
    atomic_answer: Mutex<Option<Canned>>,
    tracking_answer: Mutex<Option<Canned>>,
    atomic_calls: Mutex<Vec<AtomicCall>>,
    tracking_calls: Mutex<Vec<TrackingCall>>,
}

impl RecordingLimiter {
    pub(crate) fn new(window_size: i64, atomic: Canned, tracking: Canned) -> Self {
        Self {
            window_size,
            atomic_answer: Mutex::new(Some(atomic)),
            tracking_answer: Mutex::new(Some(tracking)),
            atomic_calls: Mutex::new(Vec::new()),
            tracking_calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn atomic_calls(&self) -> Vec<AtomicCall> {
        let calls = self
            .atomic_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }

    pub(crate) fn tracking_calls(&self) -> Vec<TrackingCall> {
        let calls = self
            .tracking_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

fn take_answer(slot: &Mutex<Option<Canned>>) -> Canned {
    let mut answer = slot.lock().unwrap_or_else(PoisonError::into_inner);
    answer
        .take()
        .expect("a canned answer is set for every call")
}

#[async_trait::async_trait]
impl AdmissionLimiter for RecordingLimiter {
    fn window_size(&self) -> i64 {
        self.window_size
    }

    async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
        parent_span: Option<ParentOtelSpan>,
    ) -> Canned {
        let mut calls = self
            .atomic_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        calls.push(AtomicCall {
            descriptors: descriptors.to_vec(),
            increments: increments.to_vec(),
            parent_span: parent_span.is_some(),
        });
        drop(calls);
        take_answer(&self.atomic_answer)
    }

    async fn should_rate_limit(
        &self,
        descriptors: &[RateLimitDescriptor],
        parent_span: Option<ParentOtelSpan>,
        read_only: bool,
    ) -> Canned {
        let mut calls = self
            .tracking_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        calls.push(TrackingCall {
            descriptors: descriptors.to_vec(),
            read_only,
            parent_span: parent_span.is_some(),
        });
        drop(calls);
        take_answer(&self.tracking_answer)
    }
}

pub(crate) struct RecordingProviders {
    model: String,
    provider: String,
    calls: Mutex<Vec<String>>,
}

impl RecordingProviders {
    pub(crate) fn new(model: &str, provider: &str) -> Self {
        Self {
            model: model.to_owned(),
            provider: provider.to_owned(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl RateLimitProviderResolver for RecordingProviders {
    fn resolve(&self, model: &str) -> (String, String) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(model.to_owned());
        drop(calls);
        (self.model.clone(), self.provider.clone())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum GroupAnswer {
    Limits(ModelGroupLimits),
    Missing,
    Unavailable,
}

pub(crate) struct FixedRouter {
    answer: GroupAnswer,
    calls: Mutex<Vec<String>>,
}

impl FixedRouter {
    pub(crate) fn new(answer: GroupAnswer) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl ModelGroupInfoSource for FixedRouter {
    fn model_group_info(
        &self,
        model: &str,
    ) -> Result<Option<ModelGroupLimits>, ModelGroupInfoError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(model.to_owned());
        drop(calls);
        match self.answer {
            GroupAnswer::Limits(group) => Ok(Some(group)),
            GroupAnswer::Missing => Ok(None),
            GroupAnswer::Unavailable => Err(ModelGroupInfoError::Unavailable {
                model: model.to_owned(),
            }),
        }
    }
}

pub(crate) struct AdmissionSetup {
    pub(crate) policy: PriorityPolicy,
    pub(crate) router: Option<GroupAnswer>,
    pub(crate) seeded: Vec<(String, CacheCounterValue)>,
    pub(crate) atomic: Canned,
    pub(crate) tracking: Canned,
}

pub(crate) struct Harness {
    pub(crate) admission: DynamicAdmission<FixedRouter>,
    pub(crate) limiter: Arc<RecordingLimiter>,
    pub(crate) providers: Arc<RecordingProviders>,
    pub(crate) cache: Arc<RecordingCache>,
}

impl Harness {
    pub(crate) fn build(setup: AdmissionSetup) -> Self {
        Self::build_at(setup, WINDOW)
    }

    pub(crate) fn build_at(setup: AdmissionSetup, limiter_window: i64) -> Self {
        let cache = Arc::new(RecordingCache::seeded(&setup.seeded));
        let limiter = Arc::new(RecordingLimiter::new(
            limiter_window,
            setup.atomic,
            setup.tracking,
        ));
        let providers = Arc::new(RecordingProviders::new("gpt-4-0613", "openai"));
        let cache_port = Arc::clone(&cache);
        let mut handler = DynamicRateLimitHandler::new(cache_port, None, setup.policy, WINDOW);
        if let Some(answer) = setup.router {
            handler.update_variables(Arc::new(FixedRouter::new(answer)));
        }
        let limiter_port = Arc::clone(&limiter);
        let providers_port = Arc::clone(&providers);
        Self {
            admission: DynamicAdmission {
                limiter: limiter_port,
                handler,
                providers: providers_port,
            },
            limiter,
            providers,
            cache,
        }
    }

    pub(crate) fn router(&self) -> &FixedRouter {
        self.admission
            .handler
            .llm_router
            .as_deref()
            .expect("the harness installed a router")
    }
}

pub(crate) fn request(model: &str) -> RequestData {
    let mut data = RequestData::new();
    data.insert(String::from("model"), Json::Text(model.to_owned()));
    data
}

pub(crate) fn with_call_id(mut data: RequestData, call_id: &str) -> RequestData {
    data.insert(
        String::from("litellm_call_id"),
        Json::Text(call_id.to_owned()),
    );
    data
}

pub(crate) fn key(metadata: &BTreeMap<String, JsonData>) -> AdmissionKey<'_> {
    AdmissionKey {
        team_metadata: None,
        key_metadata: metadata,
        parent_otel_span: None,
    }
}

pub(crate) fn key_with_span(
    metadata: &BTreeMap<String, JsonData>,
    span: ParentOtelSpan,
) -> AdmissionKey<'_> {
    AdmissionKey {
        team_metadata: None,
        key_metadata: metadata,
        parent_otel_span: Some(span),
    }
}

pub(crate) fn metadata(priority: &str) -> BTreeMap<String, JsonData> {
    BTreeMap::from([(
        String::from("priority"),
        JsonData::String(priority.to_owned()),
    )])
}

pub(crate) fn status(
    descriptor_key: &str,
    rate_limit_type: RateLimitType,
    code: &str,
    current_limit: i64,
    limit_remaining: i64,
) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit,
        limit_remaining,
        rate_limit_type,
        descriptor_key: descriptor_key.to_owned(),
        descriptor_value: Some(String::from(MODEL)),
    }
}

pub(crate) fn over_limit(descriptor_key: &str, rate_limit_type: RateLimitType) -> RateLimitStatus {
    status(descriptor_key, rate_limit_type, "OVER_LIMIT", 100, 0)
}

pub(crate) fn ok(descriptor_key: &str, rate_limit_type: RateLimitType) -> RateLimitStatus {
    status(descriptor_key, rate_limit_type, "OK", 100, 42)
}

pub(crate) fn response(overall_code: &str, statuses: Vec<RateLimitStatus>) -> RateLimitResponse {
    RateLimitResponse {
        overall_code: overall_code.to_owned(),
        statuses,
        reservation_windows: None,
    }
}

pub(crate) fn unverifiable() -> RateLimitUnverifiableError {
    RateLimitUnverifiableError {
        status_code: 503,
        detail: BTreeMap::from([(String::from("error"), String::from("redis is down"))]),
    }
}

pub(crate) fn settlement_failure() -> AdmissionLimiterError {
    AdmissionLimiterError::Settlement(SettlementError::DescriptorCount {
        descriptors: 2,
        increments: 1,
    })
}

pub(crate) fn unverifiable_failure() -> AdmissionLimiterError {
    AdmissionLimiterError::Unverifiable(unverifiable())
}

pub(crate) fn stored_response(slot: &RequestStashSlot) -> Option<RateLimitResponse> {
    let stash = slot.get()?;
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.rate_limit_response.clone()
}

pub(crate) fn stash_owner(slot: &RequestStashSlot) -> Option<String> {
    let stash = slot.get()?;
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.owner_litellm_call_id.clone()
}

pub(crate) fn header<'headers>(error: &'headers ProxyRateLimitError, name: &str) -> &'headers str {
    let headers = error.headers.as_ref().expect("the error carries headers");
    headers
        .get(name)
        .expect("the header is present")
        .to_str()
        .expect("the header is ASCII")
}

pub(crate) fn has_header(error: &ProxyRateLimitError, name: &str) -> bool {
    error
        .headers
        .as_ref()
        .is_some_and(|headers| headers.contains_key(name))
}
