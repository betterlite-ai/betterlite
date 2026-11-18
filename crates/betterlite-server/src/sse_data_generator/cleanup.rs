use std::sync::Arc;

use betterlite_models::model_registration::JsonObject;

use super::{CleanupOutcome, StreamCleanup, StreamFailure, StreamItem};
use crate::stream_keepalive::wrapper::KeepaliveSource;

impl KeepaliveSource for Result<StreamItem, StreamFailure> {
    fn hidden_params(&self) -> Option<&JsonObject> {
        self.as_ref().ok().and_then(StreamItem::hidden_params)
    }
}

pub(super) struct CleanupGuard<Key> {
    pub(super) cleanup: Arc<dyn StreamCleanup<Key>>,
    pub(super) outcome: Option<CleanupOutcome<Key>>,
}

impl<Key> CleanupGuard<Key> {
    pub(super) fn complete(&mut self, stream_completed: bool) {
        if let Some(mut outcome) = self.outcome.take() {
            outcome.stream_completed = stream_completed;
            outcome.client_disconnected = false;
            self.cleanup.finalize(outcome);
        }
    }
}

impl<Key> Drop for CleanupGuard<Key> {
    fn drop(&mut self) {
        if let Some(mut outcome) = self.outcome.take() {
            outcome.client_disconnected = !outcome.stream_completed;
            self.cleanup.finalize(outcome);
        }
    }
}
