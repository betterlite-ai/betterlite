use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use tokio::runtime::{Handle, Id};
use tokio::sync::Semaphore;

pub trait LookupClock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl LookupClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct LoopBoundSemaphore {
    pub value: usize,
    pub slot: Mutex<Option<(Id, Arc<Semaphore>)>>,
}

impl LoopBoundSemaphore {
    #[must_use]
    pub fn current(&self) -> Option<Arc<Semaphore>> {
        let runtime_id = Handle::try_current().ok()?.id();
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((bound, semaphore)) = slot.as_ref()
            && *bound == runtime_id
        {
            return Some(Arc::clone(semaphore));
        }
        let semaphore = Arc::new(Semaphore::new(self.value));
        *slot = Some((runtime_id, Arc::clone(&semaphore)));
        drop(slot);
        Some(semaphore)
    }
}

pub struct DbLookupStallTracker<Clock = SystemClock> {
    pub clock: Clock,
    pub last_hit: Mutex<Option<Instant>>,
}

impl<Clock> DbLookupStallTracker<Clock>
where
    Clock: LookupClock,
{
    pub fn record_hit(&self) {
        let now = self.clock.now();
        let mut last_hit = self.last_hit.lock().unwrap_or_else(PoisonError::into_inner);
        *last_hit = Some(now);
    }

    #[must_use]
    pub fn stalled_within(&self, window_seconds: f64) -> bool {
        let recorded = *self.last_hit.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(last_hit) = recorded else {
            return false;
        };
        self.clock
            .now()
            .saturating_duration_since(last_hit)
            .as_secs_f64()
            < window_seconds
    }
}
