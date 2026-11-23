use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_core::api::responses_poll_stream::clock::PollClock;

// the fixture's ScriptedLoop: one reading per call, then the last reading repeats
pub(super) struct ScriptedClock {
    readings: Vec<u64>,
    read: AtomicUsize,
}

impl ScriptedClock {
    pub(super) fn new(readings: &[u64]) -> Self {
        Self {
            readings: readings.to_vec(),
            read: AtomicUsize::new(0),
        }
    }
}

impl PollClock for ScriptedClock {
    fn now_millis(&self) -> u64 {
        let index = self.read.fetch_add(1, Ordering::Relaxed);
        self.readings
            .get(index)
            .or(self.readings.last())
            .copied()
            .unwrap_or_default()
    }
}
