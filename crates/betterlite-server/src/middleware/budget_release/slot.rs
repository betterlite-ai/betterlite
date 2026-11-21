use std::sync::{Arc, Mutex, PoisonError};

// The release layer takes the reservation by locking the cell itself, so there is no one-line take wrapper.
pub struct ReservationSlot<R>(pub(super) Arc<Mutex<Option<R>>>);

impl<R> ReservationSlot<R> {
    pub fn set(&self, reservation: R) {
        // LiteLLM assigns request.state.budget_reservation, so a later stamp replaces an earlier one
        let previous = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(reservation);
        if previous.is_some() {
            tracing::warn!(
                "a second budget reservation replaced the one already stamped for this request, so the first one waits for its TTL"
            );
        }
    }
}

impl<R: Send> Default for ReservationSlot<R> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

impl<R> Clone for ReservationSlot<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
