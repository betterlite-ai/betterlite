pub mod slot;
mod tests;

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use axum::http::Request;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;
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
    // the server drains this tracker at shutdown, so a release started for a dropped request finishes
    pub tasks: TaskTracker,
}

impl<R: ?Sized> Clone for BudgetReleaseLayer<R> {
    fn clone(&self) -> Self {
        Self {
            releaser: Arc::clone(&self.releaser),
            tasks: self.tasks.clone(),
        }
    }
}

impl<S, R: ?Sized> Layer<S> for BudgetReleaseLayer<R> {
    type Service = BudgetReleaseService<S, R>;

    fn layer(&self, inner: S) -> Self::Service {
        BudgetReleaseService {
            inner,
            releaser: Arc::clone(&self.releaser),
            tasks: self.tasks.clone(),
        }
    }
}

pub struct BudgetReleaseService<S, R: ?Sized> {
    inner: S,
    releaser: Arc<R>,
    tasks: TaskTracker,
}

impl<S: Clone, R: ?Sized> Clone for BudgetReleaseService<S, R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            releaser: Arc::clone(&self.releaser),
            tasks: self.tasks.clone(),
        }
    }
}

struct ReleaseGuard<R: ReservationReleaser + ?Sized + 'static> {
    slot: ReservationSlot<R::Reservation>,
    releaser: Arc<R>,
    tasks: TaskTracker,
}

impl<R: ReservationReleaser + ?Sized + 'static> ReleaseGuard<R> {
    // the release is its own tracked task, so dropping the request future cannot cancel it half way
    fn take_and_release(&self) -> Option<JoinHandle<()>> {
        let reservation = self
            .slot
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        let releaser = Arc::clone(&self.releaser);
        Some(self.tasks.spawn(async move {
            if let Err(error) = releaser.release(&reservation).await {
                tracing::error!(%error, "releasing the unclaimed budget reservation failed");
            }
        }))
    }
}

impl<R: ReservationReleaser + ?Sized + 'static> Drop for ReleaseGuard<R> {
    fn drop(&mut self) {
        drop(self.take_and_release());
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

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let slot = ReservationSlot::<R::Reservation>::default();
        req.extensions_mut().insert(slot.clone());
        let serving = self.inner.call(req);
        let guard = ReleaseGuard {
            slot,
            releaser: Arc::clone(&self.releaser),
            tasks: self.tasks.clone(),
        };
        Box::pin(async move {
            let result = serving.await;
            // awaiting the release keeps the response after the counters settle; a drop at any await leaves it to the task
            if let Some(releasing) = guard.take_and_release()
                && let Err(error) = releasing.await
            {
                tracing::error!(%error, "the budget reservation release task did not finish");
            }
            result
        })
    }
}
