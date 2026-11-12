use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, timeout_at};
pub const MAX_WAITERS: usize = 1024;
pub const UPLOAD_WAIT: Duration = Duration::from_secs(600);
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AcquireError {
    #[error("the S3 upload concurrency wait passed its deadline")]
    DeadlineExceeded,
    #[error("the S3 upload semaphore is closed")]
    Closed,
    #[error("the S3 upload limiter already holds {MAX_WAITERS} waiters")]
    Overloaded,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BoundsError {
    #[error(
        "adaptive concurrency needs 1 <= window <= ceiling, got window {window} and ceiling {ceiling}"
    )]
    Invalid { window: usize, ceiling: usize },
}
#[derive(Debug)]
pub struct UploadLimiter {
    permits: Arc<Semaphore>,
    gate: Option<Mutex<GateState>>,
}
#[derive(Debug)]
struct GateState {
    limit: usize,
    floor: usize,
    ceiling: usize,
    clean_streak: usize,
    owed: usize,
    waiters: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterSnapshot {
    pub limit: usize,
}
struct Waiting<'state> {
    state: &'state Mutex<GateState>,
}
impl UploadLimiter {
    pub fn new(window: usize, adaptive_ceiling: Option<usize>) -> Result<Self, BoundsError> {
        let permits = Arc::new(Semaphore::new(window));
        let Some(ceiling) = adaptive_ceiling else {
            return Ok(Self {
                permits,
                gate: None,
            });
        };
        if window == 0 || window > ceiling {
            return Err(BoundsError::Invalid { window, ceiling });
        }
        let gate = GateState {
            limit: window,
            floor: window,
            ceiling,
            clean_streak: 0,
            owed: 0,
            waiters: 0,
        };
        Ok(Self {
            permits,
            gate: Some(Mutex::new(gate)),
        })
    }
    pub async fn acquire(&self, deadline: Instant) -> Result<OwnedSemaphorePermit, AcquireError> {
        let _waiting = self.gate.as_ref().map(Waiting::register).transpose()?;
        loop {
            let permit = match timeout_at(deadline, Arc::clone(&self.permits).acquire_owned()).await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(_closed)) => return Err(AcquireError::Closed),
                Err(_elapsed) => return Err(AcquireError::DeadlineExceeded),
            };
            let Some(gate) = &self.gate else {
                return Ok(permit);
            };
            let mut state = gate.lock().unwrap_or_else(PoisonError::into_inner);
            if state.owed == 0 {
                return Ok(permit);
            }
            state.owed = state.owed.saturating_sub(1);
            drop(state);
            permit.forget();
        }
    }
    pub fn record(&self, throttled: bool) {
        let Some(gate) = &self.gate else {
            return;
        };
        let mut state = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let before = state.limit;
        if throttled {
            state.limit = state.floor.max(before.div_euclid(2));
            state.clean_streak = 0;
        } else {
            state.clean_streak = state.clean_streak.saturating_add(1);
            if state.clean_streak >= before && before < state.ceiling {
                state.limit = before.saturating_add(1);
                state.clean_streak = 0;
            }
        }
        let grown = state.limit.saturating_sub(before);
        let absorbed = grown.min(state.owed);
        self.permits.add_permits(grown.saturating_sub(absorbed));
        let shrunk = before.saturating_sub(state.limit);
        state.owed = state
            .owed
            .saturating_sub(absorbed)
            .saturating_add(shrunk.saturating_sub(self.permits.forget_permits(shrunk)));
    }
    #[must_use]
    pub fn snapshot(&self) -> LimiterSnapshot {
        let limit = self.gate.as_ref().map_or_else(
            || self.permits.available_permits(),
            |gate| gate.lock().unwrap_or_else(PoisonError::into_inner).limit,
        );
        LimiterSnapshot { limit }
    }
}
impl<'state> Waiting<'state> {
    fn register(state: &'state Mutex<GateState>) -> Result<Self, AcquireError> {
        let mut gate = state.lock().unwrap_or_else(PoisonError::into_inner);
        if gate.waiters >= MAX_WAITERS {
            return Err(AcquireError::Overloaded);
        }
        gate.waiters = gate.waiters.saturating_add(1);
        drop(gate);
        Ok(Self { state })
    }
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut gate = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        gate.waiters = gate.waiters.saturating_sub(1);
    }
}
mod tests;
