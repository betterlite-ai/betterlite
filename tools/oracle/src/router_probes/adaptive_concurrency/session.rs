use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use betterlite_router::adaptive_concurrency::error::AcquireError;
use betterlite_router::adaptive_concurrency::{AdaptiveConcurrencyLimiter, Permit};

use crate::router_probes::error::OperationError;

use super::wire::StateSnapshot;

pub(super) struct Session<'limiter> {
    pub(super) limiter: &'limiter AdaptiveConcurrencyLimiter,
    pub(super) acquisitions: BTreeMap<usize, Acquired<'limiter>>,
    pub(super) next_id: usize,
}

pub(super) enum Acquired<'limiter> {
    Pending(Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>>),
    Granted(Permit),
}

#[derive(Clone, Copy)]
pub(super) enum Withdraw {
    Release { id: usize },
    Cancel { id: usize },
}

impl Withdraw {
    pub(super) const fn id(self) -> usize {
        match self {
            Self::Release { id } | Self::Cancel { id } => id,
        }
    }

    pub(super) const fn rejection(self) -> &'static str {
        match self {
            Self::Release { .. } => "release names an acquisition that holds no permit",
            Self::Cancel { .. } => "cancel names an acquisition that is not waiting",
        }
    }
}

impl<'limiter> Session<'limiter> {
    pub(super) fn snapshot(&self) -> StateSnapshot {
        let snapshot = self.limiter.snapshot();
        StateSnapshot {
            limit: snapshot.limit,
            in_flight: snapshot.in_flight,
            waiters: snapshot.waiters,
            clean_streak: snapshot.clean_streak,
        }
    }

    pub(super) fn take(&mut self, id: usize) -> Result<Acquired<'limiter>, OperationError> {
        let Some(entry) = self.acquisitions.remove(&id) else {
            return Err(OperationError::InvalidRequest {
                message: "the step names an acquisition that was never started".to_owned(),
            });
        };
        Ok(entry)
    }

    pub(super) fn put(&mut self, id: usize, entry: Acquired<'limiter>) {
        if let Some(replaced) = self.acquisitions.insert(id, entry) {
            drop(replaced);
        }
    }

    pub(super) fn reject(
        &mut self,
        id: usize,
        entry: Acquired<'limiter>,
        message: &str,
    ) -> OperationError {
        self.put(id, entry);
        OperationError::InvalidRequest {
            message: message.to_owned(),
        }
    }
}
