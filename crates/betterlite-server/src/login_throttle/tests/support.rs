use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::login_throttle::{Raw, Settings};
use betterlite_context::local_ttl_cache::CacheValue;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::login_throttle::attempt::{Attempt, LoginAttempt};
use crate::login_throttle::error::LoginThrottleError;
use crate::login_throttle::keys::LoginKeys;
use crate::login_throttle::local::{Clock, LocalStore, ThrottleStores};
use crate::login_throttle::redis::{BlockTtls, FailureLimits, LoginRedis, ThrottleRedisError};
use crate::login_throttle::throttle::{LoginThrottle, ThrottleRuntime};

// ---------- fake clock ----------

#[derive(Clone, Debug, Default)]
pub struct FakeClock(Arc<Mutex<f64>>);

impl FakeClock {
    #[must_use]
    pub fn at(now: f64) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    pub fn set(&self, now: f64) {
        let mut current = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *current = now;
        drop(current);
    }

    #[must_use]
    pub fn clock(&self) -> Clock {
        let shared = Arc::clone(&self.0);
        Arc::new(move || *shared.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

// ---------- local store ----------

#[derive(Debug, Default)]
struct LocalState {
    values: BTreeMap<String, f64>,
    expiries: BTreeMap<String, f64>,
}

pub struct FakeLocalStore {
    clock: Clock,
    state: Mutex<LocalState>,
}

impl FakeLocalStore {
    #[must_use]
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            state: Mutex::new(LocalState::default()),
        }
    }

    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.purge(now);
        state.values.contains_key(key)
    }

    #[must_use]
    pub fn value(&self, key: &str) -> Option<f64> {
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.purge(now);
        state.values.get(key).copied()
    }
}

impl LocalState {
    // InMemoryCache expires lazily and only on a read; a read or a probe drops what has expired
    fn purge(&mut self, now: f64) {
        let expired: Vec<String> = self
            .expiries
            .iter()
            .filter(|(_key, expiry)| **expiry <= now)
            .map(|(key, _expiry)| key.clone())
            .collect();
        for key in &expired {
            self.values.remove(key);
            self.expiries.remove(key);
        }
    }
}

impl LocalStore for FakeLocalStore {
    fn get(&self, key: &str) -> Result<Option<CacheValue>, LoginThrottleError> {
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.purge(now);
        Ok(state.values.get(key).map(|value| CacheValue::Float(*value)))
    }

    fn set(&self, key: &str, value: CacheValue, seconds: i64) -> Result<(), LoginThrottleError> {
        let stored = match value {
            CacheValue::Float(value) => value,
            CacheValue::Integer(value) => num_traits::cast::<i64, f64>(value).unwrap_or_default(),
            _other => {
                return Err(LoginThrottleError::InvalidCount {
                    key: key.to_owned(),
                });
            }
        };
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        // InMemoryCache keeps a live TTL: the expiry moves only when there is none
        let live = state.expiries.get(key).is_some_and(|expiry| *expiry > now);
        state.values.insert(key.to_owned(), stored);
        if !live {
            let window = num_traits::cast::<i64, f64>(seconds).unwrap_or_default();
            state
                .expiries
                .insert(key.to_owned(), now.mul_add(1.0_f64, window));
        }
        drop(state);
        Ok(())
    }

    fn increment(&self, key: &str, seconds: i64) -> Result<f64, LoginThrottleError> {
        let current = match self.get(key)? {
            Some(CacheValue::Float(value)) => value,
            Some(CacheValue::Integer(value)) => {
                num_traits::cast::<i64, f64>(value).unwrap_or_default()
            }
            Some(_other) | None => 0.0_f64,
        };
        let next = current.mul_add(1.0_f64, 1.0_f64);
        self.set(key, CacheValue::Float(next), seconds)?;
        Ok(next)
    }

    fn delete(&self, key: &str) -> Result<(), LoginThrottleError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values.remove(key);
        state.expiries.remove(key);
        drop(state);
        Ok(())
    }
}

// ---------- shared store ----------

#[derive(Debug, Default)]
struct RedisState {
    counters: BTreeMap<String, i64>,
    expiries: BTreeMap<String, f64>,
    blocks: BTreeMap<String, f64>,
}

#[derive(Clone, Copy)]
struct BumpLimits {
    limit: i64,
    window_seconds: i64,
    block_seconds: i64,
}

pub struct FakeRedis {
    clock: Clock,
    faults: BTreeMap<String, String>,
    state: Mutex<RedisState>,
}

impl FakeRedis {
    #[must_use]
    pub fn new(clock: Clock, faults: &[(&str, &str)]) -> Self {
        Self {
            clock,
            faults: faults
                .iter()
                .map(|(operation, fault)| ((*operation).to_owned(), (*fault).to_owned()))
                .collect(),
            state: Mutex::new(RedisState::default()),
        }
    }

    #[must_use]
    pub fn counter(&self, key: &str) -> Option<i64> {
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.purge(now);
        state.counters.get(key).copied()
    }

    pub fn block(&self, key: &str, seconds: i64) {
        let now = (*self.clock)();
        let seconds = num_traits::cast::<i64, f64>(seconds).unwrap_or_default();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .blocks
            .insert(key.to_owned(), now.mul_add(1.0_f64, seconds));
        drop(state);
    }

    fn fault(&self, operation: &str) -> Option<ThrottleRedisError> {
        match self.faults.get(operation).map(String::as_str) {
            Some("malformed") => Some(ThrottleRedisError::MalformedBlockTtls),
            Some(_other) => Some(ThrottleRedisError::Unavailable {
                reason: "injected fault".to_owned(),
            }),
            None => None,
        }
    }

    fn record(&self, keys: &LoginKeys, limits: FailureLimits) -> BlockTtls {
        let now = (*self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let user = bump(
            &mut state,
            (&keys.pair_counter, &keys.pair_block),
            BumpLimits {
                limit: limits.user_limit,
                window_seconds: limits.window_seconds,
                block_seconds: limits.block_seconds,
            },
            now,
        );
        let source = if limits.source_limit > 0 && user == 0 {
            bump(
                &mut state,
                (&keys.source_counter, &keys.source_block),
                BumpLimits {
                    limit: limits.source_limit,
                    window_seconds: limits.window_seconds,
                    block_seconds: limits.block_seconds,
                },
                now,
            )
        } else {
            0
        };
        drop(state);
        BlockTtls { user, source }
    }
}

impl RedisState {
    fn purge(&mut self, now: f64) {
        let expired: Vec<String> = self
            .expiries
            .iter()
            .filter(|(_key, expiry)| **expiry <= now)
            .map(|(key, _expiry)| key.clone())
            .collect();
        for key in &expired {
            self.counters.remove(key);
            self.expiries.remove(key);
        }
    }
}

fn ttl_of(expiries: &BTreeMap<String, f64>, key: &str, now: f64) -> i64 {
    match expiries.get(key) {
        Some(expiry) if *expiry > now => {
            num_traits::cast::<f64, i64>(now.mul_add(-1.0_f64, *expiry))
                .unwrap_or_default()
                .max(0)
        }
        _other => -2,
    }
}

// one INCR/EXPIRE/block step of the Lua script; the keys are (counter, block)
fn bump(state: &mut RedisState, keys: (&str, &str), limits: BumpLimits, now: f64) -> i64 {
    let (count_key, block_key) = keys;
    let blocked = ttl_of(&state.blocks, block_key, now);
    if blocked > 0 {
        return blocked;
    }
    if state
        .expiries
        .get(count_key)
        .is_some_and(|expiry| *expiry <= now)
    {
        state.counters.remove(count_key);
        state.expiries.remove(count_key);
    }
    let count = state
        .counters
        .get(count_key)
        .copied()
        .unwrap_or_default()
        .saturating_add(1);
    state.counters.insert(count_key.to_owned(), count);
    if !state.expiries.contains_key(count_key) {
        let window = num_traits::cast::<i64, f64>(limits.window_seconds).unwrap_or_default();
        state
            .expiries
            .insert(count_key.to_owned(), now.mul_add(1.0_f64, window));
    }
    if count > limits.limit {
        let seconds = num_traits::cast::<i64, f64>(limits.block_seconds).unwrap_or_default();
        state
            .blocks
            .insert(block_key.to_owned(), now.mul_add(1.0_f64, seconds));
        return limits.block_seconds;
    }
    0
}

#[async_trait]
impl LoginRedis for FakeRedis {
    async fn block_ttls(&self, keys: &LoginKeys) -> Result<BlockTtls, ThrottleRedisError> {
        if let Some(error) = self.fault("block_ttls") {
            return Err(error);
        }
        let now = (*self.clock)();
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(BlockTtls {
            user: ttl_of(&state.blocks, &keys.pair_block, now),
            source: ttl_of(&state.blocks, &keys.source_block, now),
        })
    }

    async fn record_failure(
        &self,
        keys: &LoginKeys,
        limits: FailureLimits,
    ) -> Result<BlockTtls, ThrottleRedisError> {
        if let Some(error) = self.fault("record_failure") {
            return Err(error);
        }
        Ok(self.record(keys, limits))
    }

    async fn delete_counter(&self, key: &str) -> Result<(), ThrottleRedisError> {
        if let Some(error) = self.fault("delete") {
            return Err(error);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.counters.remove(key);
        state.expiries.remove(key);
        drop(state);
        Ok(())
    }
}

// ---------- warning capture ----------

#[derive(Clone, Debug, Default)]
pub struct WarningLog {
    lines: Arc<Mutex<Vec<String>>>,
}

impl WarningLog {
    #[must_use]
    pub fn subscribe(&self) -> DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(self.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::WARN)
            .finish();
        set_default(subscriber)
    }

    #[must_use]
    pub fn messages(&self) -> Vec<String> {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines
            .iter()
            .filter_map(|line| line.split_once("WARN "))
            .filter_map(|(_prefix, rest)| rest.split_once(": "))
            .map(|(_target, message)| message.to_owned())
            .collect()
    }

    #[must_use]
    pub fn drain_classified(&self) -> Vec<String> {
        let messages = self.messages();
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines.clear();
        drop(lines);
        messages.iter().map(|message| classify(message)).collect()
    }
}

impl io::Write for WarningLog {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Ok(text) = std::str::from_utf8(buffer) else {
            return Ok(buffer.len());
        };
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines.extend(text.lines().map(str::to_owned));
        drop(lines);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for WarningLog {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

// the semantic content of a warning, so the oracle compares meanings rather than wording
#[must_use]
pub fn classify(message: &str) -> String {
    let tag = if message.contains("Redis failed while counting") {
        "redis_fallback"
    } else if message.contains("Admin UI sign-in refused") {
        "refused"
    } else if message.contains("Admin UI sign-in blocked") {
        "blocked"
    } else if message.contains("counted per worker") {
        "counters_per_worker"
    } else if message.contains("is not set or not a valid list of ranges") {
        "source_limit_off"
    } else if message.contains("expected a mapping of address or range to limit") {
        "invalid_overrides_mapping"
    } else if message.contains("max_failed_login_attempts_per_source_overrides") {
        "invalid_override_value"
    } else if message.contains("Invalid trusted_proxy_ranges value") {
        "invalid_topology_shape"
    } else if message.contains("Invalid address or range") {
        "invalid_network"
    } else if message.contains("max_failed_login_attempts_per_source") {
        "invalid_source_limit"
    } else if message.contains("failed_login_window_seconds") {
        "invalid_window"
    } else if message.contains("failed_login_block_seconds") {
        "invalid_block"
    } else {
        return format!("other:{message}");
    };
    tag.to_owned()
}

// ---------- oracle input and output ----------

#[derive(Debug, Deserialize)]
pub struct OracleInput {
    pub config: Raw,
    #[serde(default)]
    pub redis: Option<RedisSpec>,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RedisSpec {
    #[serde(default)]
    pub faults: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct Phase {
    pub request: RequestSpec,
    pub now: f64,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RequestSpec {
    #[serde(default)]
    pub peer: Option<String>,
    #[serde(default)]
    pub x_forwarded_for: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Step {
    pub kind: String,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OracleOutput {
    pub phases: Vec<PhaseOutput>,
}

#[derive(Debug, Serialize)]
pub struct PhaseOutput {
    pub throttle: ThrottleView,
    pub steps: Vec<StepView>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ThrottleView {
    pub client_ip: String,
    pub source_limit: Option<i64>,
    pub user_limit: i64,
    pub window_seconds: i64,
    pub block_seconds: i64,
    pub enabled: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct StepView {
    pub step: String,
    pub outcome: String,
    pub error: Option<ErrorView>,
    pub user_block: Option<i64>,
    pub source_block: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ErrorView {
    pub code: String,
    pub r#type: String,
    pub param: Option<String>,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

// LiteLLM reads general_settings at every request; these cases convert once per phase, with an empty environment so no process variable leaks in
fn settings_for(input: &OracleInput) -> Result<Settings, String> {
    let environment = BTreeMap::<String, String>::new();
    input
        .config
        .clone()
        .convert(&environment)
        .map_err(|error| error.to_string())
}

fn network_request(spec: &RequestSpec) -> NetworkRequest<'_> {
    NetworkRequest {
        peer: spec.peer.as_deref(),
        x_forwarded_for: spec.x_forwarded_for.as_deref(),
        x_forwarded_proto: None,
        host: spec.host.as_deref(),
        scheme: "http",
    }
}

// one process's stores and shared state, as the server keeps them across requests
fn oracle_runtime(input: &OracleInput, clock: &FakeClock) -> ThrottleRuntime {
    let counters = Arc::new(FakeLocalStore::new(clock.clock()));
    let blocks = Arc::new(FakeLocalStore::new(clock.clock()));
    let counters_store: Arc<dyn LocalStore> = Arc::clone(&counters);
    let blocks_store: Arc<dyn LocalStore> = Arc::clone(&blocks);
    let stores = ThrottleStores {
        counters: counters_store,
        blocks: blocks_store,
    };
    let shared: Option<Arc<dyn LoginRedis>> = input.redis.as_ref().map(|spec| {
        let faults: Vec<(&str, &str)> = spec
            .faults
            .iter()
            .map(|(operation, fault)| (operation.as_str(), fault.as_str()))
            .collect();
        let store: Arc<dyn LoginRedis> = Arc::new(FakeRedis::new(clock.clock(), &faults));
        store
    });
    ThrottleRuntime {
        stores,
        clock: clock.clock(),
        redis: shared,
    }
}

fn throttle_view(throttle: &LoginThrottle) -> ThrottleView {
    ThrottleView {
        client_ip: throttle.client_ip.clone(),
        source_limit: throttle.source_limit,
        user_limit: throttle.user_limit,
        window_seconds: throttle.window_seconds,
        block_seconds: throttle.block_seconds,
        enabled: throttle.enabled,
    }
}

pub fn run_oracle(input: &OracleInput) -> Result<OracleOutput, String> {
    let warnings = WarningLog::default();
    let _guard = warnings.subscribe();
    let clock = FakeClock::at(0.0);
    let throttle_runtime = oracle_runtime(input, &clock);
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(run_phases(input, &warnings, &throttle_runtime, &clock))
}

async fn run_phases(
    input: &OracleInput,
    warnings: &WarningLog,
    throttle_runtime: &ThrottleRuntime,
    clock: &FakeClock,
) -> Result<OracleOutput, String> {
    let mut phases = Vec::with_capacity(input.phases.len());
    for phase in &input.phases {
        clock.set(phase.now);
        phases.push(run_phase(input, phase, warnings, throttle_runtime).await?);
    }
    Ok(OracleOutput { phases })
}

async fn run_phase(
    input: &OracleInput,
    phase: &Phase,
    warnings: &WarningLog,
    throttle_runtime: &ThrottleRuntime,
) -> Result<PhaseOutput, String> {
    let settings = settings_for(input)?;
    let request = network_request(&phase.request);
    let throttle = LoginThrottle::from_request(&request, &settings, throttle_runtime);
    let view = throttle_view(&throttle);
    let steps = run_steps(&throttle, &phase.steps).await;
    Ok(PhaseOutput {
        throttle: view,
        steps,
        warnings: warnings.drain_classified(),
    })
}

async fn run_steps(throttle: &LoginThrottle, steps: &[Step]) -> Vec<StepView> {
    let mut pending = None;
    let mut views = Vec::with_capacity(steps.len());
    for step in steps {
        views.push(run_step(throttle, step, &mut pending).await);
    }
    views
}

async fn run_step<'throttle>(
    throttle: &'throttle LoginThrottle,
    step: &Step,
    pending: &mut Option<LoginAttempt<'throttle>>,
) -> StepView {
    match step.kind.as_str() {
        "attempt" => attempt_step(throttle, step, pending).await,
        "failed" => settle_step(pending.take(), "failed").await,
        "succeeded" => settle_step(pending.take(), "succeeded").await,
        "record_failure" => record_step(throttle, step).await,
        "clear_pair" => clear_step(throttle, step).await,
        _other => StepView {
            step: step.kind.clone(),
            outcome: "unknown".to_owned(),
            ..StepView::default()
        },
    }
}

async fn attempt_step<'throttle>(
    throttle: &'throttle LoginThrottle,
    step: &Step,
    pending: &mut Option<LoginAttempt<'throttle>>,
) -> StepView {
    let username = step.username.clone().unwrap_or_default();
    *pending = None;
    let mut view = StepView {
        step: step.kind.clone(),
        ..StepView::default()
    };
    match throttle.attempt(&username).await {
        Ok(Attempt::Allowed(attempt)) => {
            view.outcome = "allowed".to_owned();
            *pending = Some(attempt);
        }
        Ok(Attempt::Refused(refused)) => {
            let exception = refused.exception();
            view.outcome = "refused".to_owned();
            view.error = Some(ErrorView {
                code: exception.code,
                r#type: exception.error_type,
                param: exception.param,
                message: exception.message,
                headers: exception.headers,
            });
        }
        Err(_error) => view.outcome = "error".to_owned(),
    }
    view
}

async fn settle_step(attempt: Option<LoginAttempt<'_>>, kind: &str) -> StepView {
    let mut view = StepView {
        step: kind.to_owned(),
        ..StepView::default()
    };
    let Some(attempt) = attempt else {
        view.outcome = "no_attempt".to_owned();
        return view;
    };
    let settled = if kind == "succeeded" {
        attempt.succeeded().await.is_ok()
    } else {
        attempt.failed().await.is_ok()
    };
    view.outcome = if settled {
        "ok".to_owned()
    } else {
        "error".to_owned()
    };
    view
}

async fn record_step(throttle: &LoginThrottle, step: &Step) -> StepView {
    let username = step.username.clone().unwrap_or_default();
    let mut view = StepView {
        step: step.kind.clone(),
        ..StepView::default()
    };
    match throttle.record_failure(&username).await {
        Ok(ttls) => {
            view.outcome = "ok".to_owned();
            view.user_block = Some(ttls.user);
            view.source_block = Some(ttls.source);
        }
        Err(_error) => view.outcome = "error".to_owned(),
    }
    view
}

async fn clear_step(throttle: &LoginThrottle, step: &Step) -> StepView {
    let username = step.username.clone().unwrap_or_default();
    let cleared = throttle.clear_pair(&username).await;
    StepView {
        step: step.kind.clone(),
        outcome: if cleared.is_ok() {
            "ok".to_owned()
        } else {
            "error".to_owned()
        },
        ..StepView::default()
    }
}
