use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::atomic_counter::value::counter_integer;
use crate::limiter_records::{CacheCounterValue, ScriptReply};

use super::SessionIterationHandler;
use super::error::SessionIterationLimitError;

// _increment_and_get: the script counts atomically across instances, and any failure falls back to the local cache
pub(crate) async fn count_after_increment(
    handler: &SessionIterationHandler,
    session_id: &str,
) -> Result<i64, SessionIterationLimitError> {
    // _make_cache_key: the hash tag keeps every key of one session on a single Redis Cluster slot
    let key = format!("{{session_iterations:{session_id}}}:count");
    let ttl_seconds = i64::try_from(handler.ttl.as_secs()).unwrap_or(i64::MAX);
    if let Some(script) = handler.increment_script.as_ref() {
        let keys = [key.clone()];
        let args = [RedisScalar::Text(ttl_seconds.to_string())];
        match script.run(&keys, &args).await {
            Ok(ScriptReply::Int(count)) => return Ok(count),
            Ok(other) => tracing::warn!(
                "MaxIterationsHandler: Redis failed, falling back to in-memory: the script answered {:?}",
                other.kind()
            ),
            Err(failure) => tracing::warn!(
                "MaxIterationsHandler: Redis failed, falling back to in-memory: {failure}"
            ),
        }
    }
    increment_in_memory(handler, &key, ttl_seconds).await
}

// _in_memory_increment: int() of the local count plus one, written back with the configured ttl
async fn increment_in_memory(
    handler: &SessionIterationHandler,
    key: &str,
    ttl_seconds: i64,
) -> Result<i64, SessionIterationLimitError> {
    let current = match handler.cache.get_cache(key, true).await {
        Some(value) => counter_integer(key, &value)?,
        None => 0,
    };
    let next = current.saturating_add(1);
    handler
        .cache
        .set_cache(key, CacheCounterValue::Int(next), ttl_seconds, true)
        .await;
    Ok(next)
}
