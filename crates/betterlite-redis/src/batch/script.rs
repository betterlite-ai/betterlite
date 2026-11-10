use std::sync::Arc;

use redis::Value;

use super::error::BatchError;
use super::operation::Settlement;
use super::replies::{namespaced, settle_pipeline};
use super::slot::Slot;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_cache::RedisCache;
use crate::redis_counters::scripts::RegisteredScript;
use crate::redis_live_client::LiveClientError;

pub(crate) struct ScriptOperation {
    pub(crate) script: Arc<RegisteredScript>,
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<RedisScalar>,
    pub(crate) slot: Arc<Slot<Value>>,
}

impl ScriptOperation {
    pub(crate) fn enqueue(&self, cache: &RedisCache, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd("EVALSHA");
        command.arg(&self.script.sha1).arg(self.keys.len());
        for key in &self.keys {
            command.arg(namespaced(cache.namespace.as_deref(), key));
        }
        for arg in &self.args {
            match arg {
                RedisScalar::Text(text) => {
                    command.arg(text);
                }
                RedisScalar::Bytes(bytes) => {
                    command.arg(bytes);
                }
                RedisScalar::Float(value) => {
                    command.arg(*value);
                }
            }
        }
        pipeline.add_command(command);
        1
    }

    pub(crate) fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        settle_pipeline(
            replies,
            |values| {
                values.first().cloned().ok_or(BatchError::MissingReply {
                    operation: "EVALSHA",
                })
            },
            |result| self.slot.complete(result),
        )
    }

    pub(crate) async fn settle_alone(&self, _cache: &RedisCache) {
        let result = self
            .script
            .run(&self.keys, &self.args)
            .await
            .map_err(BatchError::Script);
        self.slot.complete(result.map_err(Arc::new));
    }
}
