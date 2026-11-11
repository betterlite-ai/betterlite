#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_cache::backend::CacheBackend;
use betterlite_cache::configuration::cache::{Backend, Cache, CacheLifecycleError};
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::ping::HealthCheckCacheParams;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_admin::info_parse::{ClientListError, InfoValue};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::{
    RedisConnectionSettings, RedisOverrides,
};
use betterlite_redis::redis_live_client::LiveClientFactory;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::cache_routes::admin::{CacheAdmin, CacheAdminError, RedisAdmin};
use crate::cache_routes::live::LiveResponseCache;
use crate::cache_routes::{CacheRoutesState, router as cache_routes_router};

pub(crate) struct FakeLive {
    cache: Option<Arc<dyn CacheAdmin>>,
}

impl FakeLive {
    pub(crate) fn with(cache: Arc<dyn CacheAdmin>) -> Self {
        Self { cache: Some(cache) }
    }

    pub(crate) fn without() -> Self {
        Self { cache: None }
    }
}

impl LiveResponseCache for FakeLive {
    fn current(&self) -> Option<Arc<dyn CacheAdmin>> {
        self.cache.as_ref().map(Arc::clone)
    }
}

pub(crate) struct FakeCache {
    pub(crate) kind: String,
    pub(crate) params: String,
    pub(crate) health: HealthCheckCacheParams,
    pub(crate) ping: Mutex<Option<Result<bool, ()>>>,
    pub(crate) delete: Mutex<Option<Result<(), ()>>>,
    pub(crate) deleted: Mutex<Vec<Vec<String>>>,
    pub(crate) pings: AtomicUsize,
    pub(crate) writes: AtomicUsize,
    pub(crate) redis: Option<Arc<dyn RedisAdmin>>,
}

pub(crate) fn fake_cache() -> FakeCache {
    FakeCache {
        kind: "local".to_owned(),
        params: r#"{"type": "local"}"#.to_owned(),
        health: empty_health(),
        ping: Mutex::new(Some(Ok(true))),
        delete: Mutex::new(Some(Ok(()))),
        deleted: Mutex::new(Vec::new()),
        pings: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
        redis: None,
    }
}

pub(crate) fn empty_health() -> HealthCheckCacheParams {
    HealthCheckCacheParams {
        host: None,
        port: None,
        redis_kwargs: None,
        namespace: None,
        redis_version: None,
    }
}

// A working cache for a Redis-backed route test.
pub(crate) fn working_cache() -> Arc<FakeCache> {
    let redis = Arc::new(fake_redis());
    let admin: Arc<dyn RedisAdmin> = Arc::<FakeRedis>::clone(&redis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    Arc::new(cache)
}

#[async_trait]
impl CacheAdmin for FakeCache {
    fn cache_type(&self) -> String {
        self.kind.clone()
    }

    fn masked_cache_params(&self) -> String {
        self.params.clone()
    }

    fn health_check_params(&self) -> HealthCheckCacheParams {
        self.health.clone()
    }

    async fn ping(&self) -> Result<bool, CacheAdminError> {
        self.pings.fetch_add(1, Ordering::SeqCst);
        let scripted = self
            .ping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match scripted {
            Some(Ok(pinged)) => Ok(pinged),
            Some(Err(())) | None => Err(ping_failure()),
        }
    }

    async fn add_test_key(&self) -> Result<(), CacheAdminError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn delete_cache_keys(&self, keys: &[String]) -> Result<(), CacheAdminError> {
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(keys.to_vec());
        let scripted = self
            .delete
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match scripted {
            Some(Ok(())) => Ok(()),
            Some(Err(())) | None => Err(delete_failure()),
        }
    }

    fn redis_admin(&self) -> Option<Arc<dyn RedisAdmin>> {
        self.redis.as_ref().map(Arc::clone)
    }
}

pub(crate) fn ping_failure() -> CacheAdminError {
    let source = RedisAdminError::ClientList(ClientListError::OrphanToken);
    CacheAdminError::Ping {
        source: CacheLifecycleError::Ping { source },
    }
}

pub(crate) fn delete_failure() -> CacheAdminError {
    let source = RedisAdminError::ClientList(ClientListError::OrphanToken);
    CacheAdminError::Delete {
        source: CacheLifecycleError::DeleteKeys { source },
    }
}

pub(crate) struct FakeRedis {
    pub(crate) clients: Mutex<Result<Vec<IndexMap<String, String>>, ()>>,
    pub(crate) info: Mutex<Result<IndexMap<String, InfoValue>, ()>>,
    pub(crate) flush: Mutex<Result<(), ()>>,
    pub(crate) flushes: AtomicUsize,
}

pub(crate) fn fake_redis() -> FakeRedis {
    FakeRedis {
        clients: Mutex::new(Ok(Vec::new())),
        info: Mutex::new(Ok(IndexMap::new())),
        flush: Mutex::new(Ok(())),
        flushes: AtomicUsize::new(0),
    }
}

impl RedisAdmin for FakeRedis {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError> {
        let scripted = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        scripted.map_err(|()| CacheAdminError::ClientList {
            source: RedisAdminError::ClientList(ClientListError::OrphanToken),
        })
    }

    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError> {
        let scripted = self
            .info
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        scripted.map_err(|()| CacheAdminError::Info {
            source: RedisAdminError::ClientList(ClientListError::OrphanToken),
        })
    }

    fn flushall(&self) -> Result<(), CacheAdminError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        let scripted = *self.flush.lock().unwrap_or_else(PoisonError::into_inner);
        scripted.map_err(|()| CacheAdminError::Flush {
            source: RedisAdminError::ClientList(ClientListError::OrphanToken),
        })
    }
}

// Panics the way a blocking Redis call would if its worker thread aborted.
pub(crate) struct PanickingRedis;

fn panicking<T>(value: Option<T>) -> T {
    value.expect("the blocking cache call panicked")
}

impl RedisAdmin for PanickingRedis {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError> {
        Ok(panicking(None::<Vec<IndexMap<String, String>>>))
    }

    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError> {
        Ok(panicking(None::<IndexMap<String, InfoValue>>))
    }

    fn flushall(&self) -> Result<(), CacheAdminError> {
        panicking(None::<()>);
        Ok(())
    }
}

pub(crate) fn admin_router(live: FakeLive) -> Router {
    cache_routes_router(CacheRoutesState {
        cache: Arc::new(live),
    })
}

pub(crate) async fn call_text(
    router: Router,
    method: &str,
    uri: &str,
    body: &str,
    key: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = builder.body(Body::from(body.to_owned())).unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

pub(crate) async fn call_json<T>(
    router: Router,
    method: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, T)
where
    T: DeserializeOwned,
{
    let (status, text) = call_text(router, method, uri, body, None).await;
    let value = serde_json::from_str(&text).expect("a JSON response body");
    (status, value)
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorBody {
    pub(crate) error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorDetail {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) error_type: String,
    pub(crate) param: String,
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PingBody {
    pub(crate) status: String,
    pub(crate) cache_type: String,
    pub(crate) ping_response: Option<bool>,
    pub(crate) set_cache_response: Option<String>,
    pub(crate) litellm_cache_params: Option<String>,
    pub(crate) health_check_cache_params: Option<HealthCheckCacheParams>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DetailBody {
    pub(crate) detail: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatusBody {
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RedisClientsBody {
    Listed(Vec<IndexMap<String, String>>),
    Unavailable(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RedisInfoBody {
    pub(crate) num_clients: i64,
    pub(crate) clients: RedisClientsBody,
    pub(crate) info: IndexMap<String, Json>,
}

pub(crate) fn parsed_cache_params(params: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = serde_json::from_str(params).expect("the cache params JSON") else {
        panic!("the cache params are an object");
    };
    fields
}

pub(crate) struct StubBackend;

#[async_trait]
impl CacheBackend for StubBackend {
    fn default_ttl(&self) -> i64 {
        60_i64
    }
}

pub(crate) fn local_cache() -> Cache {
    Cache {
        backend: Backend::Injected(Arc::new(StubBackend)),
        cache_type: LiteLlmCacheType::Local,
        namespace: None,
        redis_flush_size: None,
        ttl: None,
        mode: CacheMode::DefaultOn,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
    }
}

pub(crate) struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

pub(crate) struct ClosedGate;

impl BreakerGate for ClosedGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

// The same overrides redis-py receives when nothing is configured.
pub(crate) fn overrides() -> RedisOverrides {
    RedisOverrides {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        gcp_service_account: None,
        gcp_ssl_ca_certs: None,
        azure_redis_ad_token: None,
        azure_client_id: None,
        azure_tenant_id: None,
        azure_client_secret: None,
        aws_iam_auth: None,
        aws_iam_user_name: None,
        aws_iam_cache_name: None,
        aws_iam_region: None,
        aws_iam_serverless: None,
        passthrough: BTreeMap::new(),
    }
}

// The settings Redis resolves when nothing is configured, before a test changes one field.
pub(crate) fn connection_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

// A cache that opens without a server: every operation fails, which is the Redis-failure path the routes answer.
pub(crate) async fn unreachable_redis_cache() -> Cache {
    let mut settings = overrides();
    settings.host = Some("127.0.0.1".to_owned());
    settings.port = Some(IntOrText::Value(1_i64));
    redis_cache_from(settings).await
}

pub(crate) async fn redis_cache_from(overrides: RedisOverrides) -> Cache {
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60_i64,
        redis_flush_size: None,
        default_major_version: 7_u32,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(ClosedGate),
    };
    let environment = BTreeMap::new();
    let redis = connect(options, &environment)
        .await
        .expect("the cache opens without a server");
    Cache {
        backend: Backend::Redis(Box::new(redis)),
        cache_type: LiteLlmCacheType::Redis,
        namespace: None,
        redis_flush_size: None,
        ttl: None,
        mode: CacheMode::DefaultOn,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
    }
}
