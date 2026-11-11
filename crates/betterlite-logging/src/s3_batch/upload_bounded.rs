use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, SemaphorePermit};
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
    kind: Kind,
}
#[derive(Debug)]
enum Kind {
    Fixed(Semaphore),
    Adaptive {
        permits: Arc<Semaphore>,
        state: Mutex<GateState>,
    },
}
#[derive(Debug)]
struct GateState {
    limit: usize,
    floor: usize,
    ceiling: usize,
    clean_streak: usize,
    owed: usize,
    waiters: usize,
    held: Vec<OwnedSemaphorePermit>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterSnapshot {
    pub limit: usize,
    pub waiters: usize,
}
pub enum UploadPermit<'limiter> {
    Fixed(SemaphorePermit<'limiter>),
    Adaptive(OwnedSemaphorePermit),
}
struct Waiting<'state> {
    state: &'state Mutex<GateState>,
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
            || {
                Ok(Self {
                    kind: Kind::Fixed(Semaphore::new(window)),
                })
            },
            |ceiling| adaptive_limiter(window, ceiling),
        )
    }
    pub async fn acquire(&self, deadline: Instant) -> Result<UploadPermit<'_>, AcquireError> {
        match &self.kind {
            Kind::Fixed(semaphore) => match timeout_at(deadline, semaphore.acquire()).await {
                Ok(Ok(permit)) => Ok(UploadPermit::Fixed(permit)),
                Ok(Err(_closed)) => Err(AcquireError::Closed),
                Err(_elapsed) => Err(AcquireError::DeadlineExceeded),
            },
            Kind::Adaptive { permits, state } => {
                let permit = adaptive_acquire(state, permits, deadline).await?;
                Ok(UploadPermit::Adaptive(permit))
            }
        }
    }
    pub fn record(&self, throttled: bool) {
        if let Kind::Adaptive { permits, state } = &self.kind {
            sample(permits, state, throttled);
        }
    }
    #[must_use]
    pub fn snapshot(&self) -> LimiterSnapshot {
        match &self.kind {
            Kind::Fixed(semaphore) => LimiterSnapshot {
                limit: semaphore.available_permits(),
                waiters: 0,
            },
            Kind::Adaptive { state, .. } => {
                let gate = lock(state);
                LimiterSnapshot {
                    limit: gate.limit,
                    waiters: gate.waiters,
                }
            }
        }
    }
}
async fn adaptive_acquire(
    state: &Mutex<GateState>,
    permits: &Arc<Semaphore>,
    deadline: Instant,
) -> Result<OwnedSemaphorePermit, AcquireError> {
    let waiting = Waiting::register(state)?;
    loop {
        let permit = acquire_permit(permits, deadline).await?;
        let mut gate = lock(state);
        if gate.owed > 0 {
            gate.owed = gate.owed.saturating_sub(1);
            gate.held.push(permit);
            drop(gate);
            continue;
        }
        drop(gate);
        drop(waiting);
        return Ok(permit);
    }
}
async fn acquire_permit(
    permits: &Arc<Semaphore>,
    deadline: Instant,
) -> Result<OwnedSemaphorePermit, AcquireError> {
    match timeout_at(deadline, Arc::clone(permits).acquire_owned()).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(_closed)) => Err(AcquireError::Closed),
        Err(_elapsed) => Err(AcquireError::DeadlineExceeded),
    }
}
fn adaptive_limiter(initial: usize, ceiling: usize) -> Result<UploadLimiter, BoundsError> {
    if initial == 0 || initial > ceiling {
        return Err(BoundsError::Invalid {
            window: initial,
            ceiling,
        });
    }
    let limiter = UploadLimiter {
        kind: Kind::Adaptive {
            permits: Arc::new(Semaphore::new(ceiling)),
            state: Mutex::new(GateState {
                limit: initial,
                floor: initial,
                ceiling,
                clean_streak: 0,
                owed: 0,
                waiters: 0,
                held: Vec::new(),
            }),
        },
    };
    if let Kind::Adaptive { permits, state } = &limiter.kind {
        rebalance(permits, state);
    }
    Ok(limiter)
}
fn sample(permits: &Arc<Semaphore>, state: &Mutex<GateState>, throttled: bool) {
    let mut gate = lock(state);
    if throttled {
        gate.limit = gate.floor.max(gate.limit.div_euclid(2));
        gate.clean_streak = 0;
        drop(gate);
        rebalance(permits, state);
        return;
    }
    gate.clean_streak = gate.clean_streak.saturating_add(1);
    if gate.clean_streak < gate.limit || gate.limit >= gate.ceiling {
        return;
    }
    gate.limit = gate.limit.saturating_add(1);
    gate.clean_streak = 0;
    drop(gate);
    rebalance(permits, state);
}
fn rebalance(permits: &Arc<Semaphore>, state: &Mutex<GateState>) {
    let mut gate = lock(state);
    let needed = gate.ceiling.saturating_sub(gate.limit);
    while gate.held.len() > needed {
        if let Some(permit) = gate.held.pop() {
            drop(permit);
        }
    }
    let missing = needed.saturating_sub(gate.held.len());
    let take = missing.min(permits.available_permits());
    if take > 0
        && let Ok(permit) =
            Arc::clone(permits).try_acquire_many_owned(u32::try_from(take).unwrap_or(u32::MAX))
    {
        gate.held.push(permit);
    }
    gate.owed = needed.saturating_sub(gate.held.len());
}
impl Waiting<'_> {
    fn register(state: &Mutex<GateState>) -> Result<Waiting<'_>, AcquireError> {
        let mut gate = lock(state);
        if gate.waiters >= MAX_WAITERS {
            return Err(AcquireError::Overloaded);
        }
        gate.waiters = gate.waiters.saturating_add(1);
        Ok(Waiting { state })
    }
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut gate = lock(self.state);
        gate.waiters = gate.waiters.saturating_sub(1);
    }
}
mod tests;
