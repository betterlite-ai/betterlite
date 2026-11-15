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
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let taken = logger.settings.batch_size.min(queue.len());
    queue.drain(..taken).collect()
}

mod tests;
