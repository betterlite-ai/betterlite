pub mod slot;
mod tests;

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use axum::http::Request;
use tower::{Layer, Service};

use self::slot::ReservationSlot;

#[async_trait]
pub trait ReservationReleaser: Send + Sync {
    type Reservation: Send + Sync + 'static;
    type Error: Error + Send + Sync + 'static;

    async fn release(&self, reservation: &Self::Reservation) -> Result<(), Self::Error>;
}

pub struct BudgetReleaseLayer<R: ?Sized> {
    pub releaser: Arc<R>,
}

impl<R: ?Sized> Clone for BudgetReleaseLayer<R> {
    fn clone(&self) -> Self {
        Self {
            releaser: Arc::clone(&self.releaser),
        }
    }
}

impl<S, R: ?Sized> Layer<S> for BudgetReleaseLayer<R> {
    type Service = BudgetReleaseService<S, R>;

    fn layer(&self, inner: S) -> Self::Service {
        BudgetReleaseService {
            inner,
            releaser: Arc::clone(&self.releaser),
        }
    }
}

pub struct BudgetReleaseService<S, R: ?Sized> {
    inner: S,
    releaser: Arc<R>,
}

impl<S: Clone, R: ?Sized> Clone for BudgetReleaseService<S, R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            releaser: Arc::clone(&self.releaser),
        }
    }
}

impl<S, B, R> Service<Request<B>> for BudgetReleaseService<S, R>
where
    S: Service<Request<B>>,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
    R: ReservationReleaser + ?Sized + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let slot = ReservationSlot::<R::Reservation>::default();
        request.extensions_mut().insert(slot.clone());
        let serving = self.inner.call(request);
        let releaser = Arc::clone(&self.releaser);
        Box::pin(async move {
            let result = serving.await;
            // taken out of the slot so a second pass finds it empty, and the guard is gone before the await
            let stamped = slot.0.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(reservation) = stamped
                && let Err(error) = releaser.release(&reservation).await
            {
                tracing::error!(%error, "releasing the unclaimed budget reservation failed");
            }
            result
        })
    }
}
