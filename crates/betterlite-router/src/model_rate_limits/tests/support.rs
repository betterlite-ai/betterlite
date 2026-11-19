use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_tokenizer::reservation::RequestBody;
use serde_json::Number;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::IoClock;
use crate::io_token_reservations::error::IoPreCallError;
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::IoReservation;
use crate::model_rate_limits::cache::{RateLimitCache, RemoteRead};
use crate::model_rate_limits::error::{ModelRateLimitError, RateLimitExceeded};
use crate::model_rate_limits::service::{IoTokenReservationHook, ModelRateLimits};
use crate::router_core::services::RouteServices;

pub(crate) const NOW_MILLIS: u64 = 1_700_000_000_000;
pub(crate) const MINUTE: &str = "22-13";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RemoteMode {
    Up,
    Breaker,
    Down,
}

// The in-memory and Redis tiers of LiteLLM's DualCache, each a key-value map the test seeds and reads back
pub(crate) struct FakeCache {
    pub(super) local: Mutex<BTreeMap<String, i64>>,
    pub(super) remote: Mutex<BTreeMap<String, i64>>,
    pub(super) mode: RemoteMode,
    pub(super) has_remote: bool,
    pub(super) remote_reads: AtomicUsize,
}

impl FakeCache {
    pub(crate) fn new() -> Self {
        Self {
            local: Mutex::new(BTreeMap::new()),
            remote: Mutex::new(BTreeMap::new()),
            mode: RemoteMode::Up,
            has_remote: true,
            remote_reads: AtomicUsize::new(0),
        }
    }

    pub(super) fn without_remote() -> Self {
        Self {
            has_remote: false,
            ..Self::new()
        }
    }

    pub(crate) fn seed_local(self, key: &str, value: i64) -> Self {
        self.local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        self
    }

    pub(super) fn seed_remote(self, key: &str, value: i64) -> Self {
        self.remote
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        self
    }

    pub(super) fn mode(self, mode: RemoteMode) -> Self {
        Self { mode, ..self }
    }

    pub(crate) fn local_value(&self, key: &str) -> Option<i64> {
        self.local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    pub(super) fn remote_value(&self, key: &str) -> Option<i64> {
        self.remote
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }
}

#[async_trait]
impl RateLimitCache for FakeCache {
    fn local(&self, key: &str) -> Option<i64> {
        self.local_value(key)
    }

    fn has_remote(&self) -> bool {
        self.has_remote
    }

    async fn remote(&self, key: &str) -> RemoteRead {
        self.remote_reads.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            RemoteMode::Up => RemoteRead::Value(self.remote_value(key)),
            RemoteMode::Breaker => RemoteRead::BreakerOpen,
            RemoteMode::Down => RemoteRead::Failed,
        }
    }

    fn increment_local(&self, key: &str, by: i64, _ttl_seconds: u64) -> Option<i64> {
        Some(bump(&self.local, key, by))
    }

    async fn increment(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64> {
        let local = self.increment_local(key, by, ttl_seconds);
        if !self.has_remote || self.mode != RemoteMode::Up {
            return local;
        }
        Some(bump(&self.remote, key, by))
    }
}

fn bump(tier: &Mutex<BTreeMap<String, i64>>, key: &str, by: i64) -> i64 {
    let mut values = tier.lock().unwrap_or_else(PoisonError::into_inner);
    let value = values.entry(key.to_owned()).or_insert(0);
    *value = value.saturating_add(by);
    let current = *value;
    drop(values);
    current
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct HookCalls {
    pub(super) reserved: Vec<IoLimitsSeen>,
    pub(super) refunds: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IoLimitsSeen {
    pub(super) itpm: Option<i64>,
    pub(super) otpm: Option<i64>,
}

// The reservation B019's ITPM/OTPM part composes: it records the call and stashes a reservation in the request
pub(super) struct FakeIo {
    pub(super) calls: Mutex<HookCalls>,
}

impl FakeIo {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(HookCalls::default()),
        }
    }
}

#[async_trait]
impl IoTokenReservationHook for FakeIo {
    async fn reserve(
        &self,
        deployment: &IoDeployment<'_>,
        request: &mut RequestBody,
    ) -> Result<Option<IoReservation>, IoPreCallError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.reserved.push(IoLimitsSeen {
            itpm: deployment.limits.itpm,
            otpm: deployment.limits.otpm,
        });
        drop(calls);
        let reservation = IoReservation {
            itpm_reserved: 1,
            otpm_reserved: 0,
            itpm_key: None,
            otpm_key: None,
        };
        request.insert(String::from("reserved"), Json::Bool(true));
        Ok(Some(reservation))
    }

    fn refund(&self, _request: &mut RequestBody) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.refunds = calls.refunds.saturating_add(1);
    }

    async fn async_refund(&self, request: &mut RequestBody) {
        self.refund(request);
    }
}

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_millis(NOW_MILLIS))
        .unwrap_or(UNIX_EPOCH)
}

pub(super) fn clock() -> IoClock {
    Arc::new(now)
}

pub(crate) fn spec(group: &str, id: &str, model: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(String::from("model"), Json::Text(model.to_owned()));
    let mut info = JsonObject::new();
    info.insert(String::from("id"), Json::Text(id.to_owned()));
    DeploymentSpec {
        model_name: group.to_owned(),
        litellm_params: params,
        model_info: info,
        extra: JsonObject::new(),
    }
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn with_param(mut dep: DeploymentSpec, field: &str, value: i64) -> DeploymentSpec {
    dep.litellm_params.insert(field.to_owned(), number(value));
    dep
}

pub(super) fn with_info(mut dep: DeploymentSpec, field: &str, value: Json) -> DeploymentSpec {
    dep.model_info.insert(field.to_owned(), value);
    dep
}

pub(crate) fn with_extra(mut dep: DeploymentSpec, field: &str, value: Json) -> DeploymentSpec {
    dep.extra.insert(field.to_owned(), value);
    dep
}

pub(super) fn tpm_cache(local: i64, remote: i64) -> FakeCache {
    let key = tpm_key("openai/gpt-4o");
    FakeCache::new()
        .seed_local(&key, local)
        .seed_remote(&key, remote)
}

pub(super) fn rpm_cache(local: i64, remote: i64) -> FakeCache {
    let key = rpm_key("openai/gpt-4o");
    FakeCache::new()
        .seed_local(&key, local)
        .seed_remote(&key, remote)
}

pub(super) fn read_count(cache: &FakeCache) -> usize {
    cache.remote_reads.load(Ordering::SeqCst)
}

pub(super) fn tiers(cache: &FakeCache) -> [Option<i64>; 2] {
    let key = tpm_key("openai/gpt-4o");
    [cache.local_value(&key), cache.remote_value(&key)]
}

pub(super) async fn check_async(
    cache: &Arc<FakeCache>,
    dep: &DeploymentSpec,
) -> Result<(), ModelRateLimitError> {
    let check = service(Arc::clone(cache), None);
    check.async_pre_call_check(dep, &mut request()).await
}

pub(super) const COEXIST_WARNING: &str =
    "deployment configures itpm/otpm alongside tpm/rpm; both limit types are enforced";

pub(super) fn coexist_deployment(with_id: bool) -> DeploymentSpec {
    let mut deployment = with_param(
        with_info(
            spec("gpt-4o", "a", "openai/gpt-4o"),
            "itpm",
            Json::Number(Number::from(10_i64)),
        ),
        "tpm",
        100,
    );
    if !with_id {
        deployment.model_info.shift_remove("id");
    }
    deployment
}

// Runs the check twice under a tracing capture and answers how many coexistence warning lines it logged
pub(super) async fn coexist_warning_lines(with_id: bool) -> usize {
    let check = service(Arc::new(FakeCache::new()), None);
    let deployment = coexist_deployment(with_id);
    let (logs, _guard) = capture_logs();
    for _request in 0..2_u8 {
        check
            .async_pre_call_check(&deployment, &mut request())
            .await
            .unwrap();
    }
    logs.text().matches(COEXIST_WARNING).count()
}

pub(super) fn request() -> RequestBody {
    RequestBody::new()
}

pub(crate) fn service(
    cache: Arc<FakeCache>,
    io: Option<Arc<dyn IoTokenReservationHook>>,
) -> ModelRateLimits {
    ModelRateLimits::new(cache, io, clock())
}

// A check over an empty cache whose io hook records to the returned handle
pub(super) fn hooked_check() -> (ModelRateLimits, Arc<FakeIo>) {
    let io = Arc::new(FakeIo::new());
    let recorder = Arc::clone(&io);
    let hook: Arc<dyn IoTokenReservationHook> = io;
    (service(Arc::new(FakeCache::new()), Some(hook)), recorder)
}

// A router whose config registered the check, as the route pipeline reads the services.
pub(crate) struct WithRateLimits {
    pub(crate) check: ModelRateLimits,
}

impl RouteServices for WithRateLimits {
    fn model_rate_limits(&self) -> Option<&ModelRateLimits> {
        Some(&self.check)
    }
}

pub(crate) fn tpm_key(model: &str) -> String {
    format!("a:{model}:tpm:{MINUTE}")
}

pub(crate) fn rpm_key(model: &str) -> String {
    format!("a:{model}:rpm:{MINUTE}")
}

pub(super) fn exceeded(error: ModelRateLimitError) -> RateLimitExceeded {
    match error {
        ModelRateLimitError::Exceeded(exceeded) => exceeded,
        other @ (ModelRateLimitError::Limit { .. } | ModelRateLimitError::IoToken(_)) => {
            panic!("expected a rate limit, got {other}")
        }
    }
}

pub(super) fn run_case_table(name: &str, cases: &[(&'static str, fn())]) {
    for (case_name, run) in cases {
        if *case_name == name {
            run();
            return;
        }
    }
    panic!("unknown case {name}");
}

// The lines the check logs while a test runs, so the warning assertions read what was emitted, not what a helper returned
#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
