pub mod error;

mod tests;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};

use self::error::{AcquireError, BoundsError};

pub const MAX_WAITERS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PutSample {
    pub throttled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub limit: u64,
    pub in_flight: u64,
    pub waiters: usize,
    pub clean_streak: u64,
}

pub struct AdaptiveConcurrencyLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
}

struct State {
    limit: u64,
    floor: u64,
    ceiling: u64,
    clean_streak: u64,
    in_flight: u64,
    waiters: VecDeque<Arc<Waiter>>,
}

struct Waiter {
    granted: AtomicBool,
    notify: Notify,
}

pub struct Permit {
    inner: Arc<Inner>,
}

impl AdaptiveConcurrencyLimiter {
    pub fn new(initial: u64, floor: u64, ceiling: u64) -> Result<Self, BoundsError> {
        if floor < 1 || floor > ceiling {
            return Err(BoundsError::Invalid { floor, ceiling });
        }
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    limit: initial.clamp(floor, ceiling),
                    floor,
                    ceiling,
                    clean_streak: 0,
                    in_flight: 0,
                    waiters: VecDeque::new(),
                }),
            }),
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.inner);
        Snapshot {
            limit: state.limit,
            in_flight: state.in_flight,
            waiters: state.waiters.len(),
            clean_streak: state.clean_streak,
        }
    }

    pub async fn acquire(&self, deadline: Instant) -> Result<Permit, AcquireError> {
        let waiter = {
            let mut state = lock(&self.inner);
            if state.in_flight < state.limit {
                state.in_flight = state.in_flight.saturating_add(1);
                return Ok(Permit {
                    inner: Arc::clone(&self.inner),
                });
            }
            if Instant::now() >= deadline {
                return Err(AcquireError::DeadlineExceeded);
            }
            if state.waiters.len() >= MAX_WAITERS {
                return Err(AcquireError::Overloaded);
            }
            let waiter = Arc::new(Waiter {
                granted: AtomicBool::new(false),
                notify: Notify::new(),
            });
            state.waiters.push_back(Arc::clone(&waiter));
            waiter
        };
        let mut guard = WaiterGuard {
            inner: Arc::clone(&self.inner),
            waiter: Arc::clone(&waiter),
            armed: true,
        };
        match timeout_at(deadline, waiter.notify.notified()).await {
            Ok(()) => {
                guard.armed = false;
                Ok(Permit {
                    inner: Arc::clone(&self.inner),
                })
            }
            Err(_elapsed) => Err(AcquireError::DeadlineExceeded),
        }
    }

    pub fn record(&self, sample: PutSample) {
        let mut state = lock(&self.inner);
        if sample.throttled {
            state.limit = state.floor.max(state.limit.div_euclid(2));
            state.clean_streak = 0;
            drop(state);
            return;
        }
        state.clean_streak = state.clean_streak.saturating_add(1);
        if state.clean_streak < state.limit || state.limit >= state.ceiling {
            drop(state);
            return;
        }
        state.limit = state.limit.saturating_add(1);
        state.clean_streak = 0;
        grant_locked(&mut state);
        drop(state);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = lock(&self.inner);
        state.in_flight = state.in_flight.saturating_sub(1);
        grant_locked(&mut state);
        drop(state);
    }
}

struct WaiterGuard {
    inner: Arc<Inner>,
    waiter: Arc<Waiter>,
    armed: bool,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = lock(&self.inner);
        if self.waiter.granted.load(Ordering::Acquire) {
            state.in_flight = state.in_flight.saturating_sub(1);
        } else {
            state
                .waiters
                .retain(|waiter| !Arc::ptr_eq(waiter, &self.waiter));
        }
        grant_locked(&mut state);
        drop(state);
    }
}

fn grant_locked(state: &mut State) {
    while state.in_flight < state.limit {
        let Some(waiter) = state.waiters.pop_front() else {
            return;
        };
        waiter.granted.store(true, Ordering::Release);
        state.in_flight = state.in_flight.saturating_add(1);
        waiter.notify.notify_one();
    }
}

fn lock(inner: &Inner) -> MutexGuard<'_, State> {
    match inner.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}
