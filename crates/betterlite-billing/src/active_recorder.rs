use std::sync::{Mutex, PoisonError};

use crate::recorder::BillingMetricsRecorder;

mod tests;

#[derive(Debug, Default)]
pub struct ActiveRecorderRegistry {
    recorder: Mutex<Option<BillingMetricsRecorder>>,
}

impl ActiveRecorderRegistry {
    pub fn set(&self, recorder: BillingMetricsRecorder) {
        let mut active = self.recorder.lock().unwrap_or_else(PoisonError::into_inner);
        if active.replace(recorder).is_some() {
            tracing::warn!(
                "Billing metrics: a new recorder replaced the active one, which will not be flushed at shutdown"
            );
        }
    }

    #[must_use]
    pub fn pop(&self) -> Option<BillingMetricsRecorder> {
        let mut active = self.recorder.lock().unwrap_or_else(PoisonError::into_inner);
        active.take()
    }
}
