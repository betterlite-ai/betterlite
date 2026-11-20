use std::sync::Arc;

use super::ports::{BufferedOutput, ReservationRelease};
use crate::sse_data_generator::types::{CleanupOutcome, StreamCleanup};

// Owns the one cleanup call: an explicit ending seals the flags, and a body dropped mid-stream runs LiteLLM's cancel branch
pub(super) struct Guard<Key> {
    pub(super) outcome: Option<CleanupOutcome<Key>>,
    pub(super) cleanup: Arc<dyn StreamCleanup<Key>>,
    pub(super) reservation: Arc<dyn ReservationRelease<Key>>,
    pub(super) buffered: Option<Arc<dyn BufferedOutput>>,
    pub(super) delivered: bool,
    pub(super) sealed: bool,
}

impl<Key> Guard<Key> {
    pub(super) const fn mark_completed(&mut self) {
        if let Some(outcome) = self.outcome.as_mut() {
            outcome.stream_completed = true;
        }
    }
}

impl<Key> Drop for Guard<Key> {
    fn drop(&mut self) {
        let Some(mut outcome) = self.outcome.take() else {
            return;
        };
        if !self.sealed {
            let withheld = self
                .buffered
                .as_ref()
                .is_some_and(|buffered| buffered.has_buffered_provider_output());
            if !self.delivered && !withheld {
                self.reservation.release_on_cancel(&outcome.key);
            }
            outcome.client_disconnected = !outcome.stream_completed;
        }
        self.cleanup.finalize(outcome);
    }
}
