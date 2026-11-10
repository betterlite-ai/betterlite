use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use redis::Value;

use super::error::BatchError;
use super::operation::Settlement;
use super::replies::{kind, namespaced, ordered, settle_pipeline};
use super::slot::Slot;
use super::{MgetValues, deduplicated};
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_cache::RedisCache;
use crate::redis_live_client::LiveClientError;

pub(crate) struct MgetOperation {
    pub(crate) keys: Vec<String>,
    pub(crate) misses: Arc<Mutex<BTreeSet<String>>>,
    pub(crate) slot: Arc<Slot<MgetValues>>,
}

impl MgetOperation {
    pub(crate) fn new(
        keys: &[String],
        misses: &Arc<Mutex<BTreeSet<String>>>,
        slot: &Arc<Slot<MgetValues>>,
    ) -> Self {
        Self {
            keys: deduplicated(keys),
            misses: Arc::clone(misses),
            slot: Arc::clone(slot),
        }
    }

    pub(crate) fn enqueue(&self, cache: &RedisCache, pipeline: &mut redis::Pipeline) -> usize {
        let mut command = redis::cmd("MGET");
        for key in &self.keys {
            command.arg(namespaced(cache.namespace.as_deref(), key));
        }
        pipeline.add_command(command);
        1
    }

    pub(crate) fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        settle_pipeline(
            replies,
            |values| self.resolve(values),
            |result| self.complete(result),
        )
    }

    fn resolve(
        &self,
        replies: &[Value],
    ) -> Result<BTreeMap<String, Option<CachedValue>>, BatchError> {
        let Some(reply) = replies.first() else {
            return Err(BatchError::MissingReply { operation: "MGET" });
        };
        let items = redis::from_redis_value_ref::<Vec<Value>>(reply).map_err(|source| {
            BatchError::MgetReply {
                kind: kind(reply),
                source,
            }
        })?;
        let mut values = BTreeMap::new();
        for (key, item) in self.keys.iter().zip(&items) {
            let stored =
                redis::from_redis_value_ref::<Option<Vec<u8>>>(item).map_err(|source| {
                    BatchError::MgetValue {
                        kind: kind(item),
                        source,
                    }
                })?;
            values.insert(key.clone(), decode_cached_value(stored.as_deref())?);
        }
        Ok(values)
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        let requested: Vec<Option<&str>> = self.keys.iter().map(|key| Some(key.as_str())).collect();
        let result = match cache.async_batch_get_cache(&requested).await {
            Ok(found) => {
                if self.keys.iter().any(|key| !found.contains_key(key)) {
                    Err(BatchError::MissingKeys {
                        requested: self.keys.len(),
                        returned: found.len(),
                    })
                } else {
                    Ok(ordered(&self.keys, found))
                }
            }
            Err(source) => Err(BatchError::Fallback(source)),
        };
        self.complete(result.map_err(Arc::new));
    }

    fn complete(&self, result: Result<BTreeMap<String, Option<CachedValue>>, Arc<BatchError>>) {
        if let Ok(values) = &result {
            let mut misses = match self.misses.lock() {
                Ok(misses) => misses,
                Err(poisoned) => poisoned.into_inner(),
            };
            misses.extend(
                values
                    .iter()
                    .filter(|(_key, value)| value.is_none())
                    .map(|(key, _value)| key.clone()),
            );
        }
        self.slot.complete(result.map(Arc::new));
    }
}
