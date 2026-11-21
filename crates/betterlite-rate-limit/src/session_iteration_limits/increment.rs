use super::SessionIterationHandler;
use super::cache::SessionIterationCache;

// _make_cache_key: the hash tag keeps every key of one session on a single Redis Cluster slot
fn cache_key(session_id: &str) -> String {
    format!("{{session_iterations:{session_id}}}:count")
}

// _increment_and_get: the script counts atomically across instances, and any failure falls back to the local cache
pub(crate) async fn count_after_increment<C>(
    handler: &SessionIterationHandler<C>,
    session_id: &str,
) -> i64
where
    C: SessionIterationCache,
{
    let key = cache_key(session_id);
    if let Some(script) = handler.script.as_ref() {
        let counted = handler
            .cache
            .run_increment_script(script, &key, handler.ttl)
            .await;
        match counted {
            Ok(count) => return count,
            Err(failure) => tracing::warn!(
                "MaxIterationsHandler: Redis failed, falling back to in-memory: {failure}"
            ),
        }
    }
    increment_in_memory(handler, &key).await
}

// _in_memory_increment: the local count plus one, written back with the configured ttl
async fn increment_in_memory<C>(handler: &SessionIterationHandler<C>, key: &str) -> i64
where
    C: SessionIterationCache,
{
    let current = handler.cache.memory_count(key).await.unwrap_or(0);
    let next = current.checked_add(1).unwrap_or(i64::MAX);
    handler.cache.memory_set_count(key, next, handler.ttl).await;
    next
}
