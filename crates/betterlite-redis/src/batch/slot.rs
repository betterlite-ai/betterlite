use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::oneshot;

use super::error::BatchError;
use super::result::{HookCall, Settled, SettledHook};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Pending,
    Completed,
    Failed,
    Cancelled,
}

pub(crate) struct SlotInner<T> {
    pub(crate) sender: Option<oneshot::Sender<Settled<T>>>,
    pub(crate) outcome: Outcome,
    pub(crate) result: Option<Settled<T>>,
    pub(crate) hooks: Vec<SettledHook<T>>,
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
}

impl<T> Slot<T> {
    pub(crate) fn channel() -> (Arc<Self>, oneshot::Receiver<Settled<T>>) {
        let (sender, receiver) = oneshot::channel();
        let inner = SlotInner {
            sender: Some(sender),
            outcome: Outcome::Pending,
            result: None,
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

    pub(crate) fn is_pending(&self) -> bool {
        let outcome = self.inner().outcome;
        match outcome {
            Outcome::Pending => true,
            Outcome::Completed | Outcome::Failed | Outcome::Cancelled => false,
        }
    }
}

impl<T: Clone> Slot<T> {
    pub(crate) fn complete(&self, result: Settled<T>) {
        let mut inner = self.inner();
        if inner.outcome != Outcome::Pending {
            return;
        }
        inner.outcome = if result.is_ok() {
            Outcome::Completed
        } else {
            Outcome::Failed
        };
        inner.result = Some(result.clone());
        if let Some(sender) = inner.sender.take() {
            // a dropped handle only means its caller stopped waiting; the operation still settles
            let _ignored = sender.send(result);
        }
    }

    pub(crate) fn settle_unresolved(&self, failure: Option<Arc<BatchError>>) {
        match failure {
            Some(error) => self.complete(Err(error)),
            None => self.inner().cancel(),
        }
    }

    pub(crate) fn pending_hooks(&self) -> Vec<HookCall>
    where
        T: Send + Sync + 'static,
    {
        let mut inner = self.inner();
        let outcome = Arc::new(
            inner
                .result
                .clone()
                .unwrap_or_else(|| Err(Arc::new(BatchError::Cancelled))),
        );
        mem::take(&mut inner.hooks)
            .into_iter()
            .map(|hook| -> HookCall {
                let settled = Arc::clone(&outcome);
                Box::new(move || hook(&settled))
            })
            .collect()
    }
}
