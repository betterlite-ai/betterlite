mod tests;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::http::Request;
use betterlite_redis::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;
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
            // LiteLLM's finally drains the scope before the request ends, so the flush runs on both paths
            let (batches, result) =
                request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, serving).await;
            batches.flush_all().await;
            result
        })
    }
}
