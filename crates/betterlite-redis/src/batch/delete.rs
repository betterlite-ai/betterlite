use std::sync::Arc;

use super::error::BatchError;
use super::replies::namespaced;
use super::slot::Slot;
use crate::redis_cache::RedisCache;

pub(crate) struct DeleteOperation {
    pub(crate) key: String,
    pub(crate) slot: Arc<Slot<()>>,
}

impl DeleteOperation {
    pub(crate) fn enqueue(&self, cache: &RedisCache, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd("DEL");
        command.arg(namespaced(cache.namespace.as_deref(), &self.key));
        pipeline.add_command(command);
        1
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        let result = cache
            .admin
            .async_delete_cache(&self.key)
            .await
            .map(|_deleted| ())
            .map_err(BatchError::Delete);
        self.slot.complete(result.map_err(Arc::new));
    }
}
