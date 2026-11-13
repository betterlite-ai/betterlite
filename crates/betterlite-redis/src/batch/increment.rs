use std::sync::Arc;

use redis::Value;

use super::error::BatchError;
use super::operation::Settlement;
use super::replies::{kind, namespaced, settle_pipeline};
use super::slot::Slot;
use crate::redis_cache::RedisCache;
use crate::redis_live_client::LiveClientError;

pub(crate) struct IncrementOperation {
    pub(crate) key: String,
    pub(crate) value: f64,
    pub(crate) ttl: Option<i64>,
    pub(crate) slot: Arc<Slot<f64>>,
}

impl IncrementOperation {
    pub(crate) fn enqueue(&self, cache: &RedisCache, pipeline: &mut redis::Pipeline) -> usize {
        let name = namespaced(cache.namespace.as_deref(), &self.key);
        let mut command = redis::cmd("INCRBYFLOAT");
        command.arg(&name).arg(self.value);
        pipeline.add_command(command);
        let Some(ttl) = self.ttl else {
            return 1;
        };
        let mut expire = redis::cmd("EXPIRE");
        expire.arg(&name).arg(ttl);
        pipeline.add_command(expire);
        2
    }

    pub(crate) fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        settle_pipeline(
            replies,
            |values| {
                let Some(reply) = values.first() else {
                    return Err(BatchError::MissingReply {
                        operation: "INCRBYFLOAT",
                    });
                };
                redis::from_redis_value_ref::<f64>(reply).map_err(|source| {
                    BatchError::IncrementReply {
                        kind: kind(reply),
                        source,
                    }
                })
            },
            |result| self.slot.complete(result),
        )
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        let result = cache
            .async_increment(&self.key, self.value, self.ttl, false)
            .await
            .map_err(BatchError::Fallback);
        self.slot.complete(result.map_err(Arc::new));
    }
}
