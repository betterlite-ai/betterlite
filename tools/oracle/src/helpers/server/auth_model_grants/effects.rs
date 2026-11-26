use std::sync::{Arc, Mutex, PoisonError};

// the source calls a step made, in order, so a case can tell a skipped check from a passing one
#[derive(Clone, Default)]
pub(super) struct Effects {
    entries: Arc<Mutex<Vec<String>>>,
}

impl Effects {
    pub(super) fn record(&self, entry: String) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn entries(&self) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
