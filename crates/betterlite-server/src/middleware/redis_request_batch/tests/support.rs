use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use futures_util::stream;
use tokio::net::TcpListener;
use tower::Service;

pub(crate) const DECLARED_KEY: &str = "batch-key";

pub(crate) type SeenBatch = Arc<Mutex<Option<Arc<RedisBatch>>>>;

pub(crate) fn take(seen: &SeenBatch) -> Arc<RedisBatch> {
    seen.lock()
        .expect("the batch record")
        .take()
        .expect("the inner service saw a request batch scope")
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn overrides(port: u16) -> RedisOverrides {
    RedisOverrides {
        host: Some("127.0.0.1".to_owned()),
        port: Some(IntOrText::Value(i64::from(port))),
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: Some(FloatOrText::Value(0.1)),
        socket_connect_timeout: Some(FloatOrText::Value(0.1)),
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

async fn cache_at(port: u16) -> Arc<RedisCache> {
    let options = RedisCacheOptions {
        overrides: overrides(port),
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(AllowGate),
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(connect(options, &env).await.expect("a cache"))
}

// A port nothing listens on: every read and flush fails at connect, which is what a flush-failure test needs.
pub(crate) async fn unreachable_cache() -> Arc<RedisCache> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a bound port");
    let address = listener.local_addr().expect("the bound address");
    drop(listener);
    cache_at(address.port()).await
}

pub(crate) fn request() -> Request<Body> {
    Request::builder()
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .expect("a request")
}

fn response() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("ok"))
        .expect("a response")
}

pub(crate) async fn text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("the response body");
    String::from_utf8(bytes.to_vec()).expect("a utf-8 body")
}

pub(crate) fn record(cache: &Arc<RedisCache>, declare: bool, seen: &SeenBatch) {
    let Some(batch) = active_request_redis_batch(cache) else {
        return;
    };
    if declare {
        drop(batch.mget(&[DECLARED_KEY.to_owned()]));
    }
    *seen.lock().expect("the batch record") = Some(batch);
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Reply {
    Responded,
    Failed,
    Streaming,
    Stalled,
}

impl Reply {
    async fn answer(self) -> Result<Response, InnerFailed> {
        match self {
            Self::Responded => Ok(response()),
            Self::Failed => Err(InnerFailed),
            Self::Streaming => Ok(Response::new(Body::from_stream(stream::pending::<
                Result<Bytes, Infallible>,
            >()))),
            Self::Stalled => pending::<Result<Response, InnerFailed>>().await,
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("the inner service failed")]
pub(crate) struct InnerFailed;

pub(crate) struct Inner {
    cache: Arc<RedisCache>,
    declare: bool,
    reply: Reply,
    seen: SeenBatch,
}

impl Inner {
    pub(crate) fn for_test(
        cache: Arc<RedisCache>,
        reply: Reply,
        declare: bool,
    ) -> (Self, SeenBatch) {
        let seen = Arc::new(Mutex::new(None));
        (
            Self {
                cache,
                declare,
                reply,
                seen: Arc::clone(&seen),
            },
            seen,
        )
    }
}

impl Service<Request<Body>> for Inner {
    type Response = Response;
    type Error = InnerFailed;
    type Future = Pin<Box<dyn Future<Output = Result<Response, InnerFailed>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), InnerFailed>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Request<Body>) -> Self::Future {
        let cache = Arc::clone(&self.cache);
        let declare = self.declare;
        let reply = self.reply;
        let seen = Arc::clone(&self.seen);
        Box::pin(async move {
            record(&cache, declare, &seen);
            reply.answer().await
        })
    }
}

pub(crate) struct NotReadyOnce<S> {
    pub(crate) inner: S,
    pub(crate) polled: bool,
}

impl<S, B> Service<Request<B>> for NotReadyOnce<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.polled {
            return self.inner.poll_ready(cx);
        }
        self.polled = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        self.inner.call(req)
    }
}
