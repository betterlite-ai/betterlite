use async_trait::async_trait;
use betterlite_models::model_registration::JsonObject;

use super::ports::ReservationRelease;
use crate::sse_data_generator::types::{
    CleanupOutcome, ItemStream, StreamCleanup, StreamFailure, StreamHooks,
};
use crate::stream_chunks::StreamItem;

// What standalone wires in place of the proxy's ports: the iterator hook is the only one LiteLLM always runs
pub struct PassThroughHooks;

#[async_trait]
impl<Key> StreamHooks<Key> for PassThroughHooks
where
    Key: Sync,
{
    fn needs_iterator_wrap(&self) -> bool {
        false
    }

    fn needs_per_chunk_hook(&self) -> bool {
        false
    }

    fn wrap_iterator(
        &self,
        items: ItemStream,
        _key: &Key,
        _request_data: &JsonObject,
    ) -> ItemStream {
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _key: &Key,
        _request_data: &JsonObject,
        _str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        Ok(item)
    }

    async fn failure_hook(
        &self,
        _key: &Key,
        _failure: &StreamFailure,
        _request_data: &JsonObject,
    ) -> Result<(), StreamFailure> {
        Ok(())
    }

    fn fire_deferred_stream_logging(&self, _request_data: &JsonObject) {}
}

pub(super) struct NoRelease;

impl<Key> ReservationRelease<Key> for NoRelease {
    fn release_on_cancel(&self, _key: &Key) {}
}

pub(super) struct NoCleanup;

impl<Key> StreamCleanup<Key> for NoCleanup {
    fn finalize(&self, _outcome: CleanupOutcome<Key>) {}
}
