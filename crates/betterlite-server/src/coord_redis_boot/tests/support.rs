#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_auth::auth_cache::{AuthCache, gateway_cache};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::sections::settings::JsonValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_client_reuse::breaker::BreakerOpen;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_router::redis_failure_policy::Settings;
use betterlite_router::routing_state_store::BreakerClock;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{set_default, with_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::coord_redis_boot::attach::{ConfigCacheTarget, UsageCacheTarget, UsageCaches};
use crate::coord_redis_boot::env_refs::{CoordSecretLookup, EnvironmentSecrets};
use crate::coord_redis_boot::init::UsageRedisProbe;
use crate::proxy_request_contracts::coordination_redis::CoordinationRedisParams;
use crate::redis_cache_wiring::build::{RedisCacheRequest, RedisCacheWiring};

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const TIMEOUT_MIN_DURATION: Duration = Duration::from_secs(60);
const TIMEOUT_LOG_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_TTL: i64 = 120;
const MAJOR_VERSION: u32 = 7;

pub(crate) struct FakeClock {
    millis: AtomicU64,
}

impl FakeClock {
    pub(crate) fn new() -> Self {
        Self {
            millis: AtomicU64::new(0),
        }
    }

    pub(crate) fn handle(self: &Arc<Self>) -> BreakerClock {
        let clock = Arc::clone(self);
        Arc::new(move || Duration::from_millis(clock.millis.load(Ordering::Relaxed)))
    }
}

fn policy() -> Settings {
    Settings {
        enabled: true,
        failure_threshold: 3,
        recovery_timeout: RECOVERY_TIMEOUT,
        timeout_min_duration: TIMEOUT_MIN_DURATION,
        timeout_log_interval: TIMEOUT_LOG_INTERVAL,
    }
}

pub(crate) struct NoopHooks;

impl ServiceHooks for NoopHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

pub(crate) fn wiring() -> RedisCacheWiring {
    RedisCacheWiring {
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoopHooks),
        policy: policy(),
        clock: Arc::new(FakeClock::new()).handle(),
        default_ttl: DEFAULT_TTL,
        default_major_version: MAJOR_VERSION,
    }
}

// A listener on port zero names a port nothing listens on once it is closed again.
pub(crate) fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let port = listener.local_addr().expect("the listener address").port();
    drop(listener);
    port
}

pub(crate) fn overrides(port: u16) -> RedisOverrides {
    let text = format!(
        r#"{{"host": "127.0.0.1", "port": {port}, "socket_timeout": 1.0, "socket_connect_timeout": 1.0}}"#
    );
    serde_json::from_value(json(&text)).expect("the overrides are valid")
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn no_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(crate) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the text is JSON")
}

pub(crate) fn params(text: &str) -> CoordinationRedisParams {
    CoordinationRedisParams::from_value(&json(text)).expect("the params are valid")
}

pub(crate) async fn standalone_cache() -> Arc<RedisCache> {
    let wire = wiring();
    let request = RedisCacheRequest {
        overrides: overrides(closed_port()),
        family: RedisCacheFamily::Standalone,
        namespace: None,
        redis_flush_size: None,
    };
    let cache = wire
        .build(request, &no_environment())
        .await
        .expect("the cache builds against a closed port");
    Arc::new(cache)
}

fn memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    Arc::new(LocalMemoryOps::new(1024, clock).expect("the memory tier builds"))
}

fn dual_cache_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1000,
        redis_delete_chunk_size: 1000,
    }
}

// The real dual cache the server hands to the spend, CLI SSO and config attachment.
pub(crate) fn dual_cache(redis: Option<Arc<RedisCache>>) -> DualCache<LocalMemoryOps> {
    DualCache::new(memory(), redis, dual_cache_settings())
}

// The real auth cache over two partition tiers, as the proxy builds it.
pub(crate) fn auth_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(memory(), memory(), None, dual_cache_settings(), 45.0_f64)
}

// A secret lookup with no manager and no OIDC provider reads the environment alone.
pub(crate) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoopOidc),
        azure_tokens: Arc::new(NoopOidc),
        clock: Arc::new(NoopOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

struct NoopOidc;

impl OidcHttp for NoopOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the coordination tests read no OIDC token".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoopOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoopOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttachCall {
    Attached { default_ttl: Option<f64> },
    Replaced,
}

pub(crate) struct Recorded {
    pub(crate) call: AttachCall,
    pub(crate) redis: Arc<RedisCache>,
}

pub(crate) struct RecordingTarget {
    calls: Mutex<Vec<Recorded>>,
}

impl RecordingTarget {
    pub(crate) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<AttachCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|recorded| recorded.call.clone())
            .collect()
    }

    pub(crate) fn caches(&self) -> Vec<Arc<RedisCache>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|recorded| Arc::clone(&recorded.redis))
            .collect()
    }

    fn record(&self, call: AttachCall, redis: Arc<RedisCache>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded { call, redis });
    }
}

impl UsageCacheTarget for RecordingTarget {
    fn attach_redis(&self, redis: Arc<RedisCache>, default_ttl: Option<f64>) {
        self.record(AttachCall::Attached { default_ttl }, redis);
    }
}

impl ConfigCacheTarget for RecordingTarget {
    fn replace_redis(&self, redis: Arc<RedisCache>) {
        self.record(AttachCall::Replaced, redis);
    }
}

pub(crate) struct RecordingCaches {
    pub(crate) spend: RecordingTarget,
    pub(crate) cli: RecordingTarget,
    config: RecordingTarget,
    pub(crate) auth: RecordingTarget,
}

impl RecordingCaches {
    pub(crate) fn new() -> Self {
        Self {
            spend: RecordingTarget::new(),
            cli: RecordingTarget::new(),
            config: RecordingTarget::new(),
            auth: RecordingTarget::new(),
        }
    }

    pub(crate) fn usage(&self) -> UsageCaches<'_> {
        UsageCaches {
            spend_counter: &self.spend,
            cli_sso: &self.cli,
            config: &self.config,
            auth: &self.auth,
        }
    }

    pub(crate) fn calls(&self) -> Vec<AttachCall> {
        let mut calls = Vec::new();
        calls.extend(self.spend.calls());
        calls.extend(self.cli.calls());
        calls.extend(self.config.calls());
        calls.extend(self.auth.calls());
        calls
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PingOutcome {
    Reachable,
    Unreachable,
    Fails,
    Hangs,
}

pub(crate) struct RecordingProbe {
    pub(crate) outcome: PingOutcome,
}

#[async_trait]
impl UsageRedisProbe for RecordingProbe {
    async fn ping(&self, _redis: &RedisCache) -> Result<bool, RedisAdminError> {
        match self.outcome {
            PingOutcome::Reachable => Ok(true),
            PingOutcome::Unreachable => Ok(false),
            PingOutcome::Fails => Err(RedisAdminError::Breaker(BreakerOpen { name: "ping" })),
            PingOutcome::Hangs => std::future::pending().await,
        }
    }
}

pub(crate) struct FakeSecrets {
    values: BTreeMap<String, SecretValue>,
    pub(crate) fails: bool,
}

impl FakeSecrets {
    pub(crate) fn new(pairs: &[(&str, SecretValue)]) -> Self {
        Self {
            values: pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
            fails: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            values: BTreeMap::new(),
            fails: true,
        }
    }
}

// A test injects named secrets and failures in front of the environment-only lookup.
impl CoordSecretLookup for FakeSecrets {
    fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError> {
        if self.fails {
            return Err(SecretLookupError::GoogleOidcFailed);
        }
        let stripped = name.replace("os.environ/", "");
        if let Some(value) = self.values.get(&stripped) {
            return Ok(Some(value.clone()));
        }
        EnvironmentSecrets.get_secret(env, name)
    }
}

#[derive(Clone)]
pub(crate) struct LogLine {
    pub(crate) level: Level,
    pub(crate) message: String,
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct LogCapture(Arc<Mutex<Vec<LogLine>>>);

impl<S: Subscriber> Layer<S> for LogCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogLine {
                level: *event.metadata().level(),
                message: visitor.0,
            });
    }
}

pub(crate) fn capture_logs<F, T>(run: F) -> (T, Vec<LogLine>)
where
    F: FnOnce() -> T,
{
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(LogCapture(Arc::clone(&lines)));
    let outcome = with_default(subscriber, run);
    let captured = lines.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (outcome, captured)
}

// A current-thread test awaits inside the guard, so the subscriber stays on its own thread.
pub(crate) async fn capture_async_logs<F, T>(run: F) -> (T, Vec<LogLine>)
where
    F: Future<Output = T>,
{
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(LogCapture(Arc::clone(&lines)));
    let guard = set_default(subscriber);
    let outcome = run.await;
    drop(guard);
    let captured = lines.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (outcome, captured)
}

pub(crate) fn messages(lines: &[LogLine], level: Level) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.level == level)
        .map(|line| line.message.clone())
        .collect()
}
