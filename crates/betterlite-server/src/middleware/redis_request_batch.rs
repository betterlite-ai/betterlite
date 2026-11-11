mod tests;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::Request;
use betterlite_redis::request_batch::batches::{POST_CALL_FLUSH_DEADLINE, RequestRedisBatches};
use betterlite_redis::request_batch::scope::{ACTIVE_REQUEST_BATCHES, request_redis_batch_scope};
use tokio::runtime::Handle;
use tower::{Layer, Service};

#[derive(Clone, Copy, Debug, Default)]
pub struct RedisBatchLayer;

impl<S> Layer<S> for RedisBatchLayer {
    type Service = RedisBatchService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RedisBatchService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct RedisBatchService<S> {
    inner: S,
}

struct FlushOnDrop(Option<Arc<RequestRedisBatches>>);

impl Drop for FlushOnDrop {
    fn drop(&mut self) {
        let Some(batches) = self.0.take() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        // a call cut off before its response head still sends the write-backs the request declared
        runtime.spawn(async move { batches.flush_all().await });
    }
}

impl<S, B> Service<Request<B>> for RedisBatchService<S>
where
    S: Service<Request<B>>,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let serving = self.inner.call(req);
        Box::pin(async move {
            let (batches, (mut guard, result)) =
                request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async move {
                    let active = ACTIVE_REQUEST_BATCHES.try_with(Arc::clone).ok();
                    (FlushOnDrop(active), serving.await)
                })
                .await;
            // LiteLLM's finally drains the scope before the request ends, so the flush runs on both paths
            batches.flush_all().await;
            guard.0 = None;
            result
        })
    }
}
