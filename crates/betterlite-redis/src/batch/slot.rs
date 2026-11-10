use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::oneshot;

use super::error::BatchError;
use super::result::{SettledHook, SettledStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Pending,
    Completed,
    Failed,
    Cancelled,
}

pub(crate) struct SlotInner<T> {
    pub(crate) sender: Option<oneshot::Sender<Result<T, Arc<BatchError>>>>,
    pub(crate) outcome: Outcome,
    pub(crate) hooks: Vec<SettledHook>,
}

pub(crate) struct Slot<T>(pub(crate) Mutex<SlotInner<T>>);

impl<T> SlotInner<T> {
    pub(crate) fn cancel(&mut self) {
        if self.outcome != Outcome::Pending {
            return;
        }
        self.outcome = Outcome::Cancelled;
        drop(self.sender.take());
    }

    fn settle(&mut self, result: Result<T, Arc<BatchError>>) {
        if self.outcome != Outcome::Pending {
            return;
        }
        self.outcome = if result.is_ok() {
            Outcome::Completed
        } else {
            Outcome::Failed
        };
        if let Some(sender) = self.sender.take() {
            // a dropped handle only means its caller stopped waiting; the operation still settles
            let _ignored = sender.send(result);
        }
    }
}

impl<T> Slot<T> {
    pub(crate) fn channel() -> (Arc<Self>, oneshot::Receiver<Result<T, Arc<BatchError>>>) {
        let (sender, receiver) = oneshot::channel();
        let inner = SlotInner {
            sender: Some(sender),
            outcome: Outcome::Pending,
            hooks: Vec::new(),
        };
        (Arc::new(Self(Mutex::new(inner))), receiver)
    }

    fn inner(&self) -> MutexGuard<'_, SlotInner<T>> {
        match self.0.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn complete(&self, result: Result<T, Arc<BatchError>>) {
        match self.0.lock() {
            Ok(mut inner) => inner.settle(result),
            Err(poisoned) => poisoned.into_inner().settle(result),
        }
    }

    pub(crate) fn settle_unresolved(&self, failure: Option<Arc<BatchError>>) {
        match failure {
            Some(error) => self.complete(Err(error)),
            None => match self.0.lock() {
                Ok(mut inner) => inner.cancel(),
                Err(poisoned) => poisoned.into_inner().cancel(),
            },
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        let outcome = self.inner().outcome;
        match outcome {
            Outcome::Pending => true,
            Outcome::Completed | Outcome::Failed | Outcome::Cancelled => false,
        }
    }

    pub(crate) fn status(&self) -> SettledStatus {
        let outcome = self.inner().outcome;
        match outcome {
            Outcome::Completed => SettledStatus::Completed,
            Outcome::Failed => SettledStatus::Failed,
            Outcome::Pending | Outcome::Cancelled => SettledStatus::Cancelled,
        }
    }

    pub(crate) fn take_hooks(&self) -> Vec<SettledHook> {
        match self.0.lock() {
            Ok(mut inner) => mem::take(&mut inner.hooks),
            Err(poisoned) => mem::take(&mut poisoned.into_inner().hooks),
        }
    }
}
