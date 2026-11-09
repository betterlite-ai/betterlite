use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use super::super::error::{Observed, StateStoreError};
use super::{RedisState, guarded, namespaced, resolve_float_ttl, truncate_seconds};
use crate::routing_state_store::FloatIncrement;
use crate::routing_value_store::RouteKey;

pub(crate) async fn increment(
    state: &RedisState,
    key: &RouteKey,
    value: f64,
    ttl_seconds: Option<f64>,
    refresh_ttl: bool,
) -> Result<Observed<f64>, StateStoreError> {
    let ttl = resolve_float_ttl(state, ttl_seconds);
    let namespaced = namespaced(state, key);
    guarded(state, "async_increment", move || async move {
        state
            .counters
            .incr_by_float_with_ttl(&namespaced, value, Some(ttl), refresh_ttl)
            .await
            .map_err(|source| StateStoreError::Counters {
                operation: "async_increment",
                source,
            })
    })
    .await
}

pub(crate) async fn increment_pipeline(
    state: &RedisState,
    entries: &[FloatIncrement],
) -> Result<Observed<Option<Vec<f64>>>, StateStoreError> {
    if entries.is_empty() {
        return Ok(Observed {
            value: None,
            warning: None,
        });
    }
    let mut operations = Vec::with_capacity(entries.len());
    for entry in entries {
        let ttl = match entry.ttl_seconds {
            None => None,
            Some(seconds) => {
                Some(truncate_seconds(seconds).ok_or(StateStoreError::InvalidTtl { seconds })?)
            }
        };
        operations.push(IncrementOperation {
            key: namespaced(state, &entry.key),
            increment_value: entry.value,
            ttl,
        });
    }
    guarded(state, "async_increment_pipeline", move || async move {
        state
            .counters
            .pipeline_increment(&operations)
            .await
            .map(Some)
            .map_err(|source| StateStoreError::Counters {
                operation: "async_increment_pipeline",
                source,
            })
    })
    .await
}
