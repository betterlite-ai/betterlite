use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::oneshot;

use super::RedisBatch;
use super::error::BatchError;
use super::slot::Slot;

pub type Settled<T> = Result<T, Arc<BatchError>>;

pub type SettledFollowUp = Pin<Box<dyn Future<Output = ()> + Send>>;

pub(crate) type HookCall = Box<dyn FnOnce() -> Option<SettledFollowUp> + Send>;

pub type SettledHook<T> = Arc<dyn Fn(&Settled<T>) -> Option<SettledFollowUp> + Send + Sync>;

pub struct BatchResult<T> {
    pub(crate) batch: Arc<RedisBatch>,
    pub(crate) slot: Arc<Slot<T>>,
    pub(crate) receiver: oneshot::Receiver<Settled<T>>,
}

impl<T> BatchResult<T> {
    pub fn on_settled(&self, hook: SettledHook<T>) {
        match self.slot.0.lock() {
            Ok(mut inner) => inner.hooks.push(hook),
            Err(poisoned) => poisoned.into_inner().hooks.push(hook),
        }
    }

    pub fn cancel(&self) {
        match self.slot.0.lock() {
            Ok(mut inner) => inner.cancel(),
            Err(poisoned) => poisoned.into_inner().cancel(),
        }
    }

    async fn wait(self) -> Settled<T> {
        if self.slot.is_pending() {
            self.batch.flush().await;
        }
        match self.receiver.await {
            Ok(result) => result,
            Err(_closed) => Err(Arc::new(BatchError::Cancelled)),
        }
    }
}

impl<T> IntoFuture for BatchResult<T>
where
    T: Send + 'static,
{
    type Output = Settled<T>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.wait())
    }
}
