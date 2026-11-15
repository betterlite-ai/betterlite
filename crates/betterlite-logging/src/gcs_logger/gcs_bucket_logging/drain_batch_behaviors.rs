use std::fmt::Debug;
use std::sync::PoisonError;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::{GcsBucketLogger, QueuedItem};

// LiteLLM takes up to batch_size items with get_nowait, so a short queue ends the drain early
pub fn drain_queue_batch<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>) -> Vec<QueuedItem>
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let limit = logger.settings.batch_size;
    let mut drained = Vec::new();
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    while drained.len() < limit {
        let Some(item) = queue.pop_front() else {
            break;
        };
        drained.push(item);
    }
    drained
}

mod tests;
