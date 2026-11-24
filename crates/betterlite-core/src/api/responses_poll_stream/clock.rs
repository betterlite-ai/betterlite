use std::time::Instant;

// the event loop clock the source reads; it only ever reads, because nothing flushes on a timer
pub trait PollClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

// B080's production clock: the process's monotonic instant, which never jumps with the wall clock
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    #[must_use]
    pub fn started() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl PollClock for SystemClock {
    fn now_millis(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}
