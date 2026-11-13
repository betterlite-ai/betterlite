use std::sync::Arc;

use serde::ser::Error as _;
use serde_json::value::RawValue;

use super::error::{BatchError, PipelineFailure};
use super::replies::namespaced;
use super::slot::Slot;
use crate::redis_cache::RedisCache;
use crate::redis_counters::increment::resolve_ttl;

pub(crate) struct SetOperation {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) ttl: Option<f64>,
    pub(crate) encode_error: Option<serde_json::Error>,
    pub(crate) slot: Arc<Slot<()>>,
}

impl SetOperation {
    pub(crate) fn enqueue(
        &mut self,
        cache: &RedisCache,
        pipeline: &mut redis::Pipeline,
    ) -> Result<usize, PipelineFailure> {
        if let Some(source) = self.encode_error.take() {
            return Err(PipelineFailure::Encode { source });
        }
        let seconds = resolve_ttl(self.ttl, cache.default_ttl)?;
        let mut command = redis::cmd("SET");
        command
            .arg(namespaced(cache.namespace.as_deref(), &self.key))
            .arg(&self.value)
            .arg("EX")
            .arg(seconds);
        pipeline.add_command(command);
        Ok(1)
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        if let Some(failed) = &self.encode_error {
            let source = serde_json::Error::custom(failed);
            self.slot
                .complete(Err(Arc::new(BatchError::Serialize { source })));
            return;
        }
        let result = match RawValue::from_string(self.value.clone()) {
            Ok(raw) => cache
                .values
                .async_set_cache_pipeline_with_ttls(&[(self.key.clone(), raw, self.ttl)])
                .await
                .map_err(BatchError::SetFallback),
            Err(source) => Err(BatchError::Serialize { source }),
        };
        self.slot.complete(result.map_err(Arc::new));
    }
}
