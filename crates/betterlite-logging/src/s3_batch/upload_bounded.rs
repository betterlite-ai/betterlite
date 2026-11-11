use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore, SemaphorePermit};
use tokio::time::{Instant, timeout_at};

// the waiter cap and the permit wait are B347's policy for a limiter shared by a whole flush
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
pub enum UploadLimiter {
    Fixed(Semaphore),
    Adaptive(AdaptiveGate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterSnapshot {
    pub limit: usize,
    pub in_flight: usize,
    pub waiters: usize,
    pub clean_streak: usize,
}

#[derive(Debug)]
pub struct AdaptiveGate {
    permits: Arc<Semaphore>,
    state: Mutex<GateState>,
}

#[derive(Debug)]
struct GateState {
    limit: usize,
    floor: usize,
    ceiling: usize,
    clean_streak: usize,
    in_flight: usize,
    // permits the gate still has to take out of circulation after the limit shrank
    owed: usize,
    waiters: usize,
    held: Vec<OwnedSemaphorePermit>,
}

pub enum UploadPermit<'gate> {
    Fixed(SemaphorePermit<'gate>),
    Adaptive(AdaptivePermit<'gate>),
}

#[derive(Debug)]
pub struct AdaptivePermit<'gate> {
    gate: &'gate AdaptiveGate,
    _permit: OwnedSemaphorePermit,
}

struct Waiting<'gate> {
    gate: &'gate AdaptiveGate,
}

fn lock(state: &Mutex<GateState>) -> MutexGuard<'_, GateState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl UploadLimiter {
    pub fn new(window: usize, adaptive_ceiling: Option<usize>) -> Result<Self, BoundsError> {
        adaptive_ceiling.map_or_else(
            || Ok(Self::Fixed(Semaphore::new(window))),
            |ceiling| AdaptiveGate::new(window, ceiling).map(Self::Adaptive),
        )
    }

    pub async fn acquire(&self, deadline: Instant) -> Result<UploadPermit<'_>, AcquireError> {
        match self {
            Self::Fixed(semaphore) => match timeout_at(deadline, semaphore.acquire()).await {
                Ok(Ok(permit)) => Ok(UploadPermit::Fixed(permit)),
                Ok(Err(_closed)) => Err(AcquireError::Closed),
                Err(_elapsed) => Err(AcquireError::DeadlineExceeded),
            },
            Self::Adaptive(gate) => gate.acquire(deadline).await.map(UploadPermit::Adaptive),
        }
    }

    pub fn record(&self, throttled: bool) {
        if let Self::Adaptive(gate) = self {
            gate.sample(throttled);
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> LimiterSnapshot {
        match self {
            Self::Fixed(semaphore) => LimiterSnapshot {
                limit: semaphore.available_permits(),
                in_flight: 0,
                waiters: 0,
                clean_streak: 0,
            },
            Self::Adaptive(gate) => gate.snapshot(),
        }
    }
}

impl AdaptiveGate {
    fn new(initial: usize, ceiling: usize) -> Result<Self, BoundsError> {
        if initial == 0 || initial > ceiling {
            return Err(BoundsError::Invalid {
                window: initial,
                ceiling,
            });
        }
        let gate = Self {
            permits: Arc::new(Semaphore::new(ceiling)),
            state: Mutex::new(GateState {
                limit: initial,
                floor: initial,
                ceiling,
                clean_streak: 0,
                in_flight: 0,
                owed: 0,
                waiters: 0,
                held: Vec::new(),
            }),
        };
        gate.rebalance();
        Ok(gate)
    }

    async fn acquire(&self, deadline: Instant) -> Result<AdaptivePermit<'_>, AcquireError> {
        let waiting = Waiting::register(self)?;
        loop {
            let permit = match timeout_at(deadline, Arc::clone(&self.permits).acquire_owned()).await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(_closed)) => return Err(AcquireError::Closed),
                Err(_elapsed) => return Err(AcquireError::DeadlineExceeded),
            };
            let mut state = lock(&self.state);
            if state.owed > 0 {
                state.owed = state.owed.saturating_sub(1);
                state.held.push(permit);
                drop(state);
                continue;
            }
            state.in_flight = state.in_flight.saturating_add(1);
            drop(state);
            drop(waiting);
            return Ok(AdaptivePermit {
                gate: self,
                _permit: permit,
            });
        }
    }

    fn sample(&self, throttled: bool) {
        let mut state = lock(&self.state);
        if throttled {
            state.limit = state.floor.max(state.limit.div_euclid(2));
            state.clean_streak = 0;
            drop(state);
            self.rebalance();
            return;
        }
        state.clean_streak = state.clean_streak.saturating_add(1);
        if state.clean_streak < state.limit || state.limit >= state.ceiling {
            return;
        }
        state.limit = state.limit.saturating_add(1);
        state.clean_streak = 0;
        drop(state);
        self.rebalance();
    }

    fn rebalance(&self) {
        let mut state = lock(&self.state);
        let needed = state.ceiling.saturating_sub(state.limit);
        while state.held.len() > needed {
            if let Some(permit) = state.held.pop() {
                drop(permit);
            }
        }
        let missing = needed.saturating_sub(state.held.len());
        let available = self.permits.available_permits();
        let take = missing.min(available);
        if take > 0
            && let Ok(permit) = Arc::clone(&self.permits)
                .try_acquire_many_owned(u32::try_from(take).unwrap_or(u32::MAX))
        {
            state.held.push(permit);
        }
        state.owed = needed.saturating_sub(state.held.len());
    }

    fn snapshot(&self) -> LimiterSnapshot {
        let state = lock(&self.state);
        LimiterSnapshot {
            limit: state.limit,
            in_flight: state.in_flight,
            waiters: state.waiters,
            clean_streak: state.clean_streak,
        }
    }
}

impl Waiting<'_> {
    fn register(gate: &AdaptiveGate) -> Result<Waiting<'_>, AcquireError> {
        let mut state = lock(&gate.state);
        if state.waiters >= MAX_WAITERS {
            return Err(AcquireError::Overloaded);
        }
        state.waiters = state.waiters.saturating_add(1);
        Ok(Waiting { gate })
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.gate.state);
        state.waiters = state.waiters.saturating_sub(1);
    }
}

impl Drop for AdaptivePermit<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.gate.state);
        state.in_flight = state.in_flight.saturating_sub(1);
    }
}

mod tests;
