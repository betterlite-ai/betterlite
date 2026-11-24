use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use betterlite_rate_limit::dynamic_limits::ports::{
    CacheReadError, CacheWriteError, DynamicLimitRouter, MinuteSetCache, RouterReadError,
};
use betterlite_rate_limit::dynamic_limits::{
    DynamicLimitRequest, DynamicRateLimitHandler, DynamicRateLimiterCache, PostCallRequest,
};
use betterlite_rate_limit::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use betterlite_rate_limit::limiter_records::LimiterClock;
use betterlite_rate_limit::provider_attribution::{ModelGroupLimits, PriorityValue};
use serde_json::Number;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

// 2023-11-14T22:13:20Z: every cache test reads this instant as the hour and minute 22-13
pub(crate) const FIXED_INSTANT: u64 = 1_700_000_000;
pub(crate) const NEXT_MINUTE: u64 = 1_700_000_060;
pub(crate) const FIXED_MINUTE: &str = "22-13";
pub(crate) const NEXT_MINUTE_LABEL: &str = "22-14";
pub(crate) const MODEL: &str = "gpt-4";
pub(crate) const MINUTE_KEY: &str = "22-13:gpt-4";
pub(crate) const KEY_HASH: &str = "key-hash";
pub(crate) const PLAIN_KEY_FIXTURE: &str = "{\"api_key\": \"key-hash\"}";
pub(crate) const KEYLESS_FIXTURE: &str = "{\"is_session_token\": false}";
pub(crate) const PROD_KEY_FIXTURE: &str =
    "{\"api_key\": \"key-hash\", \"metadata\": {\"priority\": \"prod\"}}";
pub(crate) const NUMBER_KEY_FIXTURE: &str =
    "{\"api_key\": \"key-hash\", \"metadata\": {\"priority\": 5}}";

// the clock the cache reads, fixed so a minute key is the same on every run
pub(crate) struct FixedClock(pub(crate) SystemTime);

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.0
    }
}

pub(crate) fn clock_at(seconds: u64) -> Arc<dyn LimiterClock> {
    let at = UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the fixed instant stays representable");
    Arc::new(FixedClock(at))
}

pub(crate) fn cache_over(recording: &Arc<RecordingCache>) -> DynamicRateLimiterCache {
    DynamicRateLimiterCache {
        cache: Arc::<RecordingCache>::clone(recording),
        clock: clock_at(FIXED_INSTANT),
    }
}

// one set add as the handler fires it: the minute key, the members and the ttl the cache was given
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetWrite {
    pub(crate) key: String,
    pub(crate) values: Vec<String>,
    pub(crate) ttl: i64,
}

// which of the cache's calls fails, or None for a cache that answers both
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Fault {
    #[default]
    None,
    Read,
    Write,
}

// a minute-set cache that keeps its sets and records every read and write, able to fail a read, fail a write or hold a write until released
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordingCache {
    sets: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
    reads: Arc<Mutex<Vec<String>>>,
    writes: Arc<Mutex<Vec<SetWrite>>>,
    gate: Option<Arc<Notify>>,
    fault: Fault,
}

impl RecordingCache {
    // a cache with the named fault, and a gate that holds its set add until the test releases it
    pub(crate) fn new(fault: Fault, gate: Option<Arc<Notify>>) -> Self {
        Self {
            sets: Arc::default(),
            reads: Arc::default(),
            writes: Arc::default(),
            gate,
            fault,
        }
    }

    pub(crate) fn holding(key: &str, values: &[&str]) -> Self {
        let cache = Self::default();
        cache.store(key, values);
        cache
    }

    pub(crate) fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.notify_one();
        }
    }

    pub(crate) fn store(&self, key: &str, values: &[&str]) {
        let mut sets = self.lock_sets();
        sets.insert(
            key.to_owned(),
            values.iter().copied().map(str::to_owned).collect(),
        );
    }

    pub(crate) fn read_keys(&self) -> Vec<String> {
        self.lock_reads().clone()
    }

    pub(crate) fn set_writes(&self) -> Vec<SetWrite> {
        self.lock_writes().clone()
    }

    fn lock_sets(&self) -> MutexGuard<'_, BTreeMap<String, Vec<String>>> {
        self.sets
            .lock()
            .expect("the test cache's lock is not poisoned")
    }

    fn lock_reads(&self) -> MutexGuard<'_, Vec<String>> {
        self.reads
            .lock()
            .expect("the test cache's lock is not poisoned")
    }

    fn lock_writes(&self) -> MutexGuard<'_, Vec<SetWrite>> {
        self.writes
            .lock()
            .expect("the test cache's lock is not poisoned")
    }
}

#[async_trait]
impl MinuteSetCache for RecordingCache {
    async fn async_get_cache(&self, key: &str) -> Result<Option<Vec<String>>, CacheReadError> {
        self.lock_reads().push(key.to_owned());
        if self.fault == Fault::Read {
            return Err(CacheReadError);
        }
        Ok(self.lock_sets().get(key).cloned())
    }

    async fn async_set_cache_sadd(
        &self,
        key: &str,
        values: &[String],
        ttl: i64,
    ) -> Result<(), CacheWriteError> {
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        self.lock_writes().push(SetWrite {
            key: key.to_owned(),
            values: values.to_vec(),
            ttl,
        });
        if self.fault == Fault::Write {
            return Err(CacheWriteError);
        }
        self.lock_sets()
            .entry(key.to_owned())
            .or_default()
            .extend(values.iter().cloned());
        Ok(())
    }
}

// a Router whose three reads answer from fields the test sets, recording what was asked
#[derive(Clone, Debug)]
pub(crate) struct FakeRouter {
    info: Result<Option<ModelGroupLimits>, RouterReadError>,
    usage: Result<(Option<i64>, Option<i64>), RouterReadError>,
    model_info: Option<String>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeRouter {
    pub(crate) fn answering(
        limits: (Option<i64>, Option<i64>),
        usage: (Option<i64>, Option<i64>),
    ) -> Self {
        let (tpm, rpm) = limits;
        Self {
            info: Ok(Some(ModelGroupLimits { rpm, tpm })),
            usage: Ok(usage),
            model_info: None,
            calls: Arc::default(),
        }
    }

    pub(crate) fn with_model_info(mut self, model_group: &str) -> Self {
        self.model_info = Some(model_group.to_owned());
        self
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the test router's lock is not poisoned")
            .clone()
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .expect("the test router's lock is not poisoned")
            .push(call);
    }
}

impl Default for FakeRouter {
    fn default() -> Self {
        Self {
            info: Ok(None),
            usage: Ok((None, None)),
            model_info: None,
            calls: Arc::default(),
        }
    }
}

#[async_trait]
impl DynamicLimitRouter for FakeRouter {
    async fn get_model_group_info(
        &self,
        model_group: &str,
    ) -> Result<Option<ModelGroupLimits>, RouterReadError> {
        self.record(format!("info:{model_group}"));
        self.info
    }

    async fn get_model_group_usage(
        &self,
        model_group: &str,
    ) -> Result<(Option<i64>, Option<i64>), RouterReadError> {
        self.record(format!("usage:{model_group}"));
        self.usage
    }

    fn get_model_info(&self, id: &str) -> Option<String> {
        self.record(format!("model:{id}"));
        self.model_info.clone()
    }
}

// a native resolver that names every model after itself with the openai provider
pub(crate) struct Native;

impl ProviderResolution for Native {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Custom(String::from("openai")),
            api_base: None,
            api_key: None,
        })
    }
}

// a user_api_key_dict as the hooks read it, deserialized from the fields the test cares about
pub(crate) fn principal(fixture: &str) -> PrincipalFacts {
    serde_json::from_str(fixture).expect("the fixture is a principal record")
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn default_policy() -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings::default(),
        reservation: None,
    }
}

pub(crate) fn prod_reservation(value: PriorityValue) -> BTreeMap<String, PriorityValue> {
    BTreeMap::from([(String::from("prod"), value)])
}

pub(crate) fn cache_holding(values: &[&str]) -> Arc<RecordingCache> {
    Arc::new(RecordingCache::holding(MINUTE_KEY, values))
}

// the cache a test's minute set holds: absent for None, an empty set for Some(0), one member per project otherwise
pub(crate) fn cache_counting(active: Option<usize>) -> Arc<RecordingCache> {
    let Some(count) = active else {
        return Arc::default();
    };
    let mut members = Vec::new();
    for index in 0..count {
        members.push(format!("key-{index}"));
    }
    let borrowed: Vec<&str> = members.iter().map(String::as_str).collect();
    Arc::new(RecordingCache::holding(MINUTE_KEY, &borrowed))
}

pub(crate) const fn request<'request>(
    model: Option<&'request str>,
    principal: &'request PrincipalFacts,
) -> DynamicLimitRequest<'request> {
    DynamicLimitRequest { model, principal }
}

pub(crate) const fn post_call<'request>(
    model_id: &'request str,
    principal: &'request PrincipalFacts,
) -> PostCallRequest<'request> {
    PostCallRequest {
        model_id,
        principal,
    }
}

// the handler under one test, with the cache, router and reservation the test chose
pub(crate) struct Harness {
    pub(crate) cache: Arc<RecordingCache>,
    pub(crate) router: FakeRouter,
    pub(crate) reservation: Option<BTreeMap<String, PriorityValue>>,
    pub(crate) with_router: bool,
}

impl Harness {
    pub(crate) fn handler(&self) -> DynamicRateLimitHandler {
        let cache: Arc<dyn MinuteSetCache> = Arc::<RecordingCache>::clone(&self.cache);
        let llm_router = self
            .with_router
            .then(|| -> Arc<dyn DynamicLimitRouter> { Arc::new(self.router.clone()) });
        handler_over(cache, llm_router, self.policy())
    }

    pub(crate) fn policy(&self) -> PriorityPolicy {
        PriorityPolicy {
            settings: PrioritySettings::default(),
            reservation: self.reservation.clone(),
        }
    }
}

pub(crate) const fn harness_with(
    router: FakeRouter,
    cache: Arc<RecordingCache>,
    reservation: Option<BTreeMap<String, PriorityValue>>,
) -> Harness {
    Harness {
        cache,
        router,
        reservation,
        with_router: true,
    }
}

pub(crate) fn harness_without_router(router: FakeRouter) -> Harness {
    Harness {
        cache: Arc::default(),
        router,
        reservation: None,
        with_router: false,
    }
}

pub(crate) fn handler_over(
    cache: Arc<dyn MinuteSetCache>,
    llm_router: Option<Arc<dyn DynamicLimitRouter>>,
    policy: PriorityPolicy,
) -> DynamicRateLimitHandler {
    DynamicRateLimitHandler {
        internal_usage_cache: Arc::new(DynamicRateLimiterCache {
            cache,
            clock: clock_at(FIXED_INSTANT),
        }),
        llm_router,
        policy,
        native: Arc::new(Native),
        router: None,
        tasks: TaskTracker::new(),
    }
}

// the failures check_available_usage turns into five Nones, one harness each
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    InfoRead,
    UsageRead,
    MissingRouter,
    CacheRead,
    NoActiveProjects,
    InfiniteWeight,
}

pub(crate) fn failing_harness(failure: Failure) -> Harness {
    let mut router = FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0)));
    let mut cache: Arc<RecordingCache> = Arc::default();
    let mut reservation: Option<BTreeMap<String, PriorityValue>> = None;
    let mut with_router = true;
    match failure {
        Failure::InfoRead => router.info = Err(RouterReadError::GroupInfo),
        Failure::UsageRead => router.usage = Err(RouterReadError::GroupUsage),
        Failure::MissingRouter => with_router = false,
        Failure::CacheRead => cache = Arc::new(RecordingCache::new(Fault::Read, None)),
        Failure::NoActiveProjects => cache = cache_holding(&[]),
        Failure::InfiniteWeight => {
            cache = cache_holding(&[KEY_HASH]);
            reservation = Some(prod_reservation(PriorityValue::Percent(f64::INFINITY)));
        }
    }
    Harness {
        cache,
        router,
        reservation,
        with_router,
    }
}
