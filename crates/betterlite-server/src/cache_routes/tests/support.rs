#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_cache::configuration::cache::CacheLifecycleError;
use betterlite_cache::types::ping::HealthCheckCacheParams;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_admin::info_parse::ClientListError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use super::redis_support::{FakeRedis, fake_redis};
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
        health: HealthCheckCacheParams {
            host: None,
            port: None,
            redis_kwargs: None,
            namespace: None,
            redis_version: None,
        },
        ping: Mutex::new(Some(Ok(true))),
        delete: Mutex::new(Some(Ok(()))),
        deleted: Mutex::new(Vec::new()),
        pings: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
        redis: None,
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
