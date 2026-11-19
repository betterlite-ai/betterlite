use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::pending;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_live_client::LiveClientError;
use chrono::{DateTime, TimeZone as _, Utc};
use redis::RedisError;
use tokio::runtime::{Builder, Runtime};
use tokio::task::{Id, JoinSet, yield_now};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::{AuthCache, gateway_cache, payload};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::clock::InvalidationClock;
use crate::auth_invalidation::error::InvalidationError;
use crate::auth_invalidation::ports::{
    AuthInvalidationCache, DeferredDelete, RedisMarker, SpendCounterCache, UsageInvalidationCache,
};
use crate::auth_invalidation::transport::InvalidationTransport;
use crate::auth_pubsub::endpoint::{
    AuthPubSub, AuthPubSubSubscription, PubSubMessage, TransportError,
};
use crate::auth_pubsub::eviction::{CacheError, LocalEviction};
use crate::auth_pubsub::message::Invalidation;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(crate) const FIXED_NOW_MILLIS: i64 = 1_000_500;
pub(crate) const FIXED_NOW: f64 = 1000.5_f64;
pub(crate) const MANAGEMENT_TTL: f64 = 45.0_f64;
pub(crate) const ACCESS_TTL: f64 = 600.0_f64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Call {
    AuthWrite {
        key: String,
        payload: Json,
        ttl: Option<f64>,
    },
    AuthDeleteSync(String),
    AuthDeleteAsync(String),
    AuthDeletePreCall(String),
    AuthDeleteMemory(String),
    AuthDeleteRedis(String),
    UsageDeleteMemory(String),
    UsageDeleteAsync(String),
    SpendSetMemory {
        key: String,
        value: f64,
        ttl: f64,
    },
    SpendSetRedis {
        key: String,
        value: f64,
        ttl: f64,
    },
    SpendRedisSkipped(String),
    SpendRedisDeleted(String),
    Publish {
        key: String,
        new_value: Option<f64>,
        ttl: Option<f64>,
    },
    Evict {
        keys: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    AuthWrite,
    AuthDeleteSync,
    AuthDeleteAsync,
    AuthDeleteMemory,
    AuthDeleteRedis,
    UsageDeleteMemory,
    UsageDeleteAsync,
    UsagePending,
    SpendSetMemory,
    SpendSetRedis,
    SpendSetRedisPending,
    SpendDeleteRedis,
    PreCallUnqueued,
    PreCallFail,
    ApplyLocally,
}

pub(crate) struct Recorder {
    calls: Mutex<Vec<Call>>,
    faults: Mutex<Vec<Fault>>,
    handle: Option<Arc<dyn RedisMarker>>,
    management_ttl: Mutex<f64>,
}

impl Recorder {
    pub(crate) fn new(handle: Option<Arc<dyn RedisMarker>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            faults: Mutex::new(Vec::new()),
            handle,
            management_ttl: Mutex::new(MANAGEMENT_TTL),
        })
    }

    pub(crate) fn fail(&self, fault: Fault) {
        lock(&self.faults).push(fault);
    }

    pub(crate) fn set_management_ttl(&self, ttl: f64) {
        *lock(&self.management_ttl) = ttl;
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }

    pub(crate) fn record(&self, call: Call) {
        lock(&self.calls).push(call);
    }

    fn has(&self, fault: Fault) -> bool {
        lock(&self.faults).contains(&fault)
    }

    fn pre_call_mode(&self) -> DeferredDelete {
        if self.has(Fault::PreCallUnqueued) {
            DeferredDelete::Unqueued
        } else {
            DeferredDelete::Settled
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl AuthInvalidationCache for Recorder {
    fn redis_handle(&self, _key: &str) -> Option<Arc<dyn RedisMarker>> {
        self.handle.clone()
    }

    fn management_ttl(&self) -> f64 {
        *lock(&self.management_ttl)
    }

    async fn write_management(
        &self,
        key: &str,
        payload: &Json,
        ttl: Option<f64>,
    ) -> Result<(), InvalidationError> {
        self.record(Call::AuthWrite {
            key: key.to_owned(),
            payload: payload.clone(),
            ttl,
        });
        if self.has(Fault::AuthWrite) {
            return Err(invalidation_failure());
        }
        Ok(())
    }

    fn delete_sync(&self, key: &str) -> Result<(), InvalidationError> {
        self.record(Call::AuthDeleteSync(key.to_owned()));
        if self.has(Fault::AuthDeleteSync) {
            return Err(invalidation_failure());
        }
        Ok(())
    }

    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError> {
        self.record(Call::AuthDeleteAsync(key.to_owned()));
        if self.has(Fault::AuthDeleteAsync) {
            return Err(invalidation_failure());
        }
        Ok(())
    }

    async fn delete_pre_call(&self, key: &str) -> Result<DeferredDelete, InvalidationError> {
        self.record(Call::AuthDeletePreCall(key.to_owned()));
        if self.has(Fault::PreCallFail) {
            return Err(invalidation_failure());
        }
        Ok(self.pre_call_mode())
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        self.record(Call::AuthDeleteMemory(key.to_owned()));
        if self.has(Fault::AuthDeleteMemory) {
            return Err(memory_failure());
        }
        Ok(())
    }

    fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError> {
        self.record(Call::AuthDeleteRedis(key.to_owned()));
        if self.has(Fault::AuthDeleteRedis) {
            return Err(admin_failure());
        }
        Ok(())
    }
}

#[async_trait]
impl UsageInvalidationCache for Recorder {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        self.handle.clone()
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        self.record(Call::UsageDeleteMemory(key.to_owned()));
        if self.has(Fault::UsageDeleteMemory) {
            return Err(memory_failure());
        }
        Ok(())
    }

    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError> {
        if self.has(Fault::UsagePending) {
            pending::<()>().await;
        }
        self.record(Call::UsageDeleteAsync(key.to_owned()));
        if !self.has(Fault::UsageDeleteAsync) {
            return Ok(());
        }
        Err(invalidation_failure())
    }
}

#[async_trait]
impl SpendCounterCache for Recorder {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        self.handle.clone()
    }

    fn set_memory(&self, key: &str, value: f64, ttl: f64) -> Result<(), BackendError> {
        self.record(Call::SpendSetMemory {
            key: key.to_owned(),
            value,
            ttl,
        });
        if self.has(Fault::SpendSetMemory) {
            return Err(memory_failure());
        }
        Ok(())
    }

    async fn set_redis(
        &self,
        key: &str,
        value: f64,
        ttl: f64,
    ) -> Result<(), RedisCacheValuesError> {
        if self.has(Fault::SpendSetRedisPending) {
            pending::<()>().await;
        }
        if self.handle.is_none() {
            self.record(Call::SpendRedisSkipped(key.to_owned()));
            return Ok(());
        }
        self.record(Call::SpendSetRedis {
            key: key.to_owned(),
            value,
            ttl,
        });
        if self.has(Fault::SpendSetRedis) {
            return Err(values_failure());
        }
        Ok(())
    }

    async fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError> {
        if self.handle.is_some() {
            self.record(Call::SpendRedisDeleted(key.to_owned()));
            if self.has(Fault::SpendDeleteRedis) {
                return Err(admin_failure());
            }
            return Ok(());
        }
        self.record(Call::SpendRedisSkipped(key.to_owned()));
        Ok(())
    }
}

#[async_trait]
impl InvalidationTransport for Recorder {
    async fn publish(&self, cache_key: &str, new_value: Option<f64>, ttl: Option<f64>) {
        self.record(Call::Publish {
            key: cache_key.to_owned(),
            new_value,
            ttl,
        });
    }

    async fn evict_and_broadcast(&self, cache_keys: &[String], local: &dyn LocalEviction) {
        self.record(Call::Evict {
            keys: cache_keys.to_vec(),
        });
        if !self.has(Fault::ApplyLocally) {
            return;
        }
        for cache_key in cache_keys {
            let _outcome = local.apply(Invalidation {
                cache_key: cache_key.clone(),
                new_value: None,
                ttl: None,
            });
        }
    }
}

pub(crate) struct Marker;

impl RedisMarker for Marker {}

pub(crate) fn marker() -> Arc<dyn RedisMarker> {
    Arc::new(Marker)
}

pub(crate) fn invalidation_failure() -> InvalidationError {
    InvalidationError::Memory(Box::new(memory_failure()))
}

pub(crate) fn memory_failure() -> BackendError {
    BackendError::Unsupported("scripted")
}

pub(crate) fn values_failure() -> RedisCacheValuesError {
    RedisCacheValuesError::TtlOutOfRange { seconds: 60.0_f64 }
}

pub(crate) fn admin_failure() -> RedisAdminError {
    RedisAdminError::LiveClient(LiveClientError::Redis(RedisError::from(io::Error::other(
        "scripted",
    ))))
}

pub(crate) struct FixedClock;

impl InvalidationClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(FIXED_NOW_MILLIS)
            .single()
            .expect("the fixed instant is valid")
    }
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

fn clock_at(seconds: f64) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || seconds)
}

pub(crate) fn local_memory() -> Arc<LocalMemoryOps> {
    let memory =
        LocalMemoryOps::new(1024, clock_at(FIXED_NOW)).expect("the local memory tier builds");
    Arc::new(memory)
}

fn cache_settings(memory_ttl: Option<f64>) -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(crate) fn auth_cache(
    memory_ttl: Option<f64>,
    management_fallback: f64,
) -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(
        local_memory(),
        local_memory(),
        None,
        cache_settings(memory_ttl),
        management_fallback,
    )
}

pub(crate) fn dual_cache(memory_ttl: Option<f64>) -> DualCache<LocalMemoryOps> {
    DualCache::new(local_memory(), None, cache_settings(memory_ttl))
}

#[derive(Default)]
pub(crate) struct RecordingEndpoint {
    published: Mutex<Vec<String>>,
}

impl RecordingEndpoint {
    pub(crate) fn published(&self) -> Vec<String> {
        lock(&self.published).clone()
    }
}

impl AuthPubSub for RecordingEndpoint {
    type Subscription = RecordingSubscription;

    fn channel(&self) -> String {
        "test-channel".to_owned()
    }

    async fn publish(&self, message: &str) -> Result<(), TransportError> {
        lock(&self.published).push(message.to_owned());
        Ok(())
    }

    async fn subscribe(&self) -> Result<Self::Subscription, TransportError> {
        Ok(RecordingSubscription)
    }
}

pub(crate) struct RecordingSubscription;

impl AuthPubSubSubscription for RecordingSubscription {
    async fn next_message(
        &mut self,
        _timeout: Duration,
    ) -> Result<Option<PubSubMessage>, TransportError> {
        Ok(None)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingEviction {
    applied: Mutex<Vec<String>>,
}

impl RecordingEviction {
    pub(crate) fn applied(&self) -> Vec<String> {
        lock(&self.applied).clone()
    }
}

impl LocalEviction for RecordingEviction {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError> {
        lock(&self.applied).push(invalidation.cache_key);
        Ok(())
    }
}

pub(crate) async fn settle() {
    yield_now().await;
    yield_now().await;
    yield_now().await;
    yield_now().await;
}

// an id for a load another task owns; the task is cancelled at once, since only its identity is used
pub(crate) fn other_task_id() -> Id {
    let runtime = runtime();
    runtime.block_on(async {
        let mut tasks = JoinSet::new();
        let handle = tasks.spawn(pending::<()>());
        let id = handle.id();
        tasks.abort_all();
        id
    })
}

pub(crate) fn builder(auth: &Arc<Recorder>, usage: Option<&Arc<Recorder>>) -> AuthInvalidation {
    let auth_handle: Arc<Recorder> = Arc::clone(auth);
    let auth_port: Arc<dyn AuthInvalidationCache> = auth_handle;
    let spend_handle: Arc<Recorder> = Arc::clone(auth);
    let spend_port: Arc<dyn SpendCounterCache> = spend_handle;
    let transport_handle: Arc<Recorder> = Arc::clone(auth);
    let transport_port: Arc<dyn InvalidationTransport> = transport_handle;
    let usage_port: Option<Arc<dyn UsageInvalidationCache>> = match usage {
        Some(recorder) => {
            let usage_handle: Arc<Recorder> = Arc::clone(recorder);
            let port: Arc<dyn UsageInvalidationCache> = usage_handle;
            Some(port)
        }
        None => None,
    };
    AuthInvalidation {
        auth: auth_port,
        usage: usage_port,
        spend: spend_port,
        transport: transport_port,
        clock: Arc::new(FixedClock),
        access_group_ttl: ACCESS_TTL,
    }
}

pub(crate) fn team(team_id: &str, alias: Option<&str>) -> LiteLLMTeamTableCachedObj {
    let mut fields = BTreeMap::new();
    fields.insert("team_id".to_owned(), JsonData::String(team_id.to_owned()));
    if let Some(alias) = alias {
        fields.insert("team_alias".to_owned(), JsonData::String(alias.to_owned()));
    }
    LiteLLMTeamTableCachedObj::try_from(JsonData::Object(fields)).expect("the team row parses")
}

pub(crate) fn access_group(access_group_id: &str, name: &str) -> AccessGroupTable {
    let text = format!(r#"{{"access_group_id":"{access_group_id}","access_group_name":"{name}"}}"#);
    serde_json::from_str(&text).expect("the access group row parses")
}

pub(crate) fn key_auth(request_route: &str) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    fields.insert(
        "token".to_owned(),
        JsonData::String("sk-uncached".to_owned()),
    );
    fields.insert(
        "request_route".to_owned(),
        JsonData::String(request_route.to_owned()),
    );
    fields.insert(
        "budget_reservation".to_owned(),
        JsonData::Object(BTreeMap::new()),
    );
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key auth builds")
}

pub(crate) fn encoded_team(team: &LiteLLMTeamTableCachedObj) -> Json {
    payload::encode(team).expect("the team encodes")
}

pub(crate) fn field<'payload>(payload: &'payload Json, name: &str) -> Option<&'payload Json> {
    match payload {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn number(payload: &Json, name: &str) -> Option<f64> {
    match field(payload, name) {
        Some(Json::Number(value)) => value.as_f64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CapturedEvent {
    pub(crate) level: String,
    pub(crate) subject: Option<String>,
}

struct CaptureLayer(Arc<Mutex<Vec<CapturedEvent>>>);

impl<Inner: Subscriber> Layer<Inner> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_invalidation")
        {
            return;
        }
        let mut visitor = SubjectVisitor(None);
        event.record(&mut visitor);
        let level = match *event.metadata().level() {
            Level::WARN => "warn",
            Level::DEBUG => "debug",
            Level::ERROR => "error",
            Level::INFO => "info",
            Level::TRACE => "trace",
        };
        lock(&self.0).push(CapturedEvent {
            level: level.to_owned(),
            subject: visitor.0,
        });
    }
}

struct SubjectVisitor(Option<String>);

impl Visit for SubjectVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "subject" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

pub(crate) fn capture<T, F>(operation: F) -> (T, Vec<CapturedEvent>)
where
    F: FnOnce() -> T,
{
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = registry().with(CaptureLayer(Arc::clone(&events)));
    let output = with_default(subscriber, operation);
    let captured = lock(&events).clone();
    (output, captured)
}

pub(crate) fn warned_subjects(events: &[CapturedEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.level == "warn")
        .filter_map(|event| event.subject.clone())
        .collect()
}

pub(crate) fn call_keys(calls: &[Call]) -> Vec<String> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::AuthWrite { key, .. }
            | Call::AuthDeleteSync(key)
            | Call::AuthDeleteAsync(key)
            | Call::AuthDeletePreCall(key)
            | Call::AuthDeleteMemory(key)
            | Call::AuthDeleteRedis(key)
            | Call::UsageDeleteMemory(key)
            | Call::UsageDeleteAsync(key)
            | Call::SpendRedisSkipped(key)
            | Call::SpendRedisDeleted(key) => Some(key.clone()),
            Call::SpendSetMemory { key, .. }
            | Call::SpendSetRedis { key, .. }
            | Call::Publish { key, .. } => Some(key.clone()),
            Call::Evict { keys } => keys.first().cloned(),
        })
        .collect()
}
