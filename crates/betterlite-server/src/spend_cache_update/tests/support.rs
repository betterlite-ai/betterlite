use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_cache::{AuthCache, gateway_cache};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use betterlite_redis::request_batch::batches::RequestRedisBatches;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;
use chrono::{DateTime, FixedOffset, TimeZone as _};
use serde_json::json;
use tempfile::NamedTempFile;
use tokio::net::TcpListener;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::failure_hook::alerts::ProxyAlerts;
use crate::failure_hook::ports::{
    AlertingClient, BudgetAlertKind, CallInfo, Clock, FailureService, ServiceLogging, SlackAlert,
    SlackAlerting, UsageCache,
};

use super::super::alerts::SpendBudgetAlerts;
use super::super::cache::{ArmedSpendRead, SpendCache};
use super::super::{SpendCacheUpdateError, UpdateDeps};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// sha256("sk-test-token"), the key the api-key branch looks up after hashing a plaintext sk- token
pub(crate) const HASHED_TEST_TOKEN: &str =
    "389faa3c31e10c880a5b4a506725160fd5165f1cd85e65619d12bcfde5296e9b";

pub(crate) fn json_from(text: &str) -> Json {
    serde_json::from_str(text).expect("the cached json fixture parses")
}

pub(crate) struct WriteCall {
    pub entries: Vec<(String, Json)>,
    pub ttl: f64,
    pub local_only: bool,
}

#[derive(Default)]
pub(crate) struct FakeState {
    pub cached: BTreeMap<String, Json>,
    pub auth: Option<UserAPIKeyAuth>,
    pub auth_requests: Vec<String>,
    pub declared: Vec<Vec<String>>,
    pub resolves: usize,
    pub reads: Vec<Vec<String>>,
    pub fail_reads: bool,
    pub fail_resolve: bool,
    pub armable: bool,
    pub management_ttl: f64,
    pub writes: Vec<WriteCall>,
}

impl FakeState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn with_cached(mut self, cached: BTreeMap<String, Json>) -> Self {
        self.cached = cached;
        self
    }

    #[must_use]
    pub(crate) fn failing_reads(mut self) -> Self {
        self.fail_reads = true;
        self
    }

    #[must_use]
    pub(crate) fn failing_resolve(mut self) -> Self {
        self.fail_resolve = true;
        self
    }

    #[must_use]
    pub(crate) fn armable(mut self) -> Self {
        self.armable = true;
        self
    }

    #[must_use]
    pub(crate) fn with_auth(mut self, auth: UserAPIKeyAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    #[must_use]
    pub(crate) fn with_management_ttl(mut self, ttl: f64) -> Self {
        self.management_ttl = ttl;
        self
    }
}

pub(crate) struct FakeCache {
    state: Arc<Mutex<FakeState>>,
}

impl FakeCache {
    pub(crate) fn new(state: FakeState) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, FakeState> {
        lock(&self.state)
    }
}

#[async_trait]
impl SpendCache for FakeCache {
    fn management_ttl(&self) -> f64 {
        self.state().management_ttl
    }

    fn declare(
        &self,
        keys: &[String],
        _batches: &RequestRedisBatches,
    ) -> Result<Option<ArmedSpendRead>, SpendCacheUpdateError> {
        let mut state = self.state();
        if !state.armable {
            return Ok(None);
        }
        state.declared.push(keys.to_vec());
        Ok(Some(ArmedSpendRead {
            keys: keys.to_vec(),
            reads: Vec::new(),
        }))
    }

    async fn read(
        &self,
        keys: &[String],
        _local_only: bool,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        let mut state = self.state();
        state.reads.push(keys.to_vec());
        if state.fail_reads {
            return Err(SpendCacheUpdateError::CacheRead);
        }
        Ok(keys
            .iter()
            .map(|key| state.cached.get(key).cloned())
            .collect())
    }

    async fn resolve(
        &self,
        armed: ArmedSpendRead,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        let mut state = self.state();
        state.resolves += 1;
        if state.fail_resolve {
            return Err(SpendCacheUpdateError::CacheRead);
        }
        Ok(armed
            .keys
            .iter()
            .map(|key| state.cached.get(key).cloned())
            .collect())
    }

    async fn auth(&self, key: &str) -> Result<Option<UserAPIKeyAuth>, SpendCacheUpdateError> {
        let mut state = self.state();
        state.auth_requests.push(key.to_owned());
        Ok(state.auth.clone())
    }

    async fn write(&self, entries: &[(String, Json)], ttl: f64, local_only: bool) {
        let mut state = self.state();
        state.writes.push(WriteCall {
            entries: entries.to_vec(),
            ttl,
            local_only,
        });
    }
}

#[derive(Default)]
pub(crate) struct RecordingAlerts {
    pub sent: Mutex<Vec<CallInfo>>,
}

impl RecordingAlerts {
    pub(crate) fn sent(&self) -> Vec<CallInfo> {
        lock(&self.sent).clone()
    }
}

#[async_trait]
impl SpendBudgetAlerts for RecordingAlerts {
    async fn projected_limit_exceeded(&self, info: CallInfo) {
        lock(&self.sent).push(info);
    }
}

pub(crate) struct FixedClock(pub DateTime<FixedOffset>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

pub(crate) fn june_15_clock() -> Arc<dyn Clock> {
    let noon = FixedOffset::east_opt(0)
        .expect("utc is a valid offset")
        .with_ymd_and_hms(2024, 6, 15, 12, 0, 0)
        .single()
        .expect("the fixture date exists");
    Arc::new(FixedClock(noon))
}

pub(crate) fn update_deps(
    cache: Arc<dyn SpendCache>,
    alerts: Arc<dyn SpendBudgetAlerts>,
    tasks: &TaskTracker,
) -> UpdateDeps {
    UpdateDeps {
        cache,
        alerts,
        tasks: tasks.clone(),
        clock: june_15_clock(),
    }
}

pub(crate) fn memory() -> Arc<LocalMemoryOps> {
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

pub(crate) fn local_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(memory(), memory(), None, dual_cache_settings(), 45.0_f64)
}

pub(crate) async fn unreachable_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free local port");
    let port = listener.local_addr().expect("the bound address").port();
    drop(listener);
    let redis = connect(
        unreachable_options(port),
        &BTreeMap::<String, String>::new(),
    )
    .await
    .expect("the cache builds against a closed port");
    gateway_cache(
        memory(),
        memory(),
        Some(Arc::new(redis)),
        dual_cache_settings(),
        45.0_f64,
    )
}

fn unreachable_options(port: u16) -> RedisCacheOptions {
    let overrides: RedisOverrides = serde_json::from_value(json!({
        "host": "127.0.0.1",
        "port": port,
        "socket_timeout": 0.05,
        "socket_connect_timeout": 0.05,
    }))
    .expect("the unreachable overrides parse");
    RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(AllowGate),
    }
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

pub(crate) async fn in_request_scope<Operation, Output>(operation: Operation) -> Output
where
    Operation: Future<Output = Output>,
{
    request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, operation)
        .await
        .1
}

pub(crate) fn capture_logs(level: tracing::Level) -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().expect("the log file");
    let subscriber = fmt()
        .without_time()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log handle")))
        .finish();
    (log, set_default(subscriber))
}

pub(crate) fn written(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).expect("the captured log reads")
}

pub(crate) fn key_auth(spend: i64, soft_budget: Option<i64>, cooldown: bool) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::from([
        (
            "token".to_owned(),
            JsonData::String("cached-token".to_owned()),
        ),
        ("key_alias".to_owned(), JsonData::String("alias".to_owned())),
        ("user_id".to_owned(), JsonData::String("user-1".to_owned())),
        (
            "spend".to_owned(),
            JsonData::Number(serde_json::Number::from(spend)),
        ),
        ("soft_budget_cooldown".to_owned(), JsonData::Bool(cooldown)),
    ]);
    if let Some(soft_budget) = soft_budget {
        fields.insert(
            "soft_budget".to_owned(),
            JsonData::Number(serde_json::Number::from(soft_budget)),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the auth fixture builds")
}

#[derive(Default)]
pub(crate) struct RecordingSlack {
    pub budget: Mutex<Vec<(BudgetAlertKind, CallInfo)>>,
}

impl RecordingSlack {
    pub(crate) fn budget_calls(&self) -> Vec<(BudgetAlertKind, CallInfo)> {
        lock(&self.budget).clone()
    }
}

#[async_trait]
impl SlackAlerting for RecordingSlack {
    fn alerting_configured(&self) -> bool {
        true
    }

    async fn failed_tracking_alert(&self, _error_message: &str, _failing_model: &str) {}

    async fn budget_alerts(&self, kind: BudgetAlertKind, user_info: &CallInfo) {
        lock(&self.budget).push((kind, user_info.clone()));
    }

    async fn send_alert(&self, _alert: SlackAlert<'_>) {}

    async fn response_taking_too_long(&self, _data: Option<JsonMap>) {}
}

pub(crate) struct QuietUsageCache;

#[async_trait]
impl UsageCache for QuietUsageCache {
    async fn set_local(&self, _key: &str, _value: &str, _ttl: Duration) {}
}

pub(crate) struct QuietServiceLogging;

#[async_trait]
impl ServiceLogging for QuietServiceLogging {
    async fn async_service_failure_hook(
        &self,
        _service: FailureService,
        _duration: Duration,
        _error: &str,
        _call_type: &str,
    ) {
    }
}

pub(crate) fn proxy_alerts(slack: Arc<RecordingSlack>) -> ProxyAlerts {
    ProxyAlerts {
        alerting: Some(vec![AlertingClient::Slack]),
        alert_types: Vec::new(),
        alerting_threshold: Duration::from_secs(60),
        proxy_base_url: None,
        usage_cache: Arc::new(QuietUsageCache),
        slack,
        email: None,
        service_logging: Arc::new(QuietServiceLogging),
        sentry: None,
        redactor: None,
        clock: june_15_clock(),
        tasks: TaskTracker::new(),
    }
}
