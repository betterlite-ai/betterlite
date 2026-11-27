use super::ToolsetCache;

pub fn invalidate_toolset_cache<C>(cache: &C, toolset_id: Option<&str>)
where
    C: ToolsetCache,
{
    let evicted = toolset_id.map_or_else(
        || cache.remove_where(&|key| key.starts_with("toolset_")),
        |id| {
            cache.remove_where(&|key| {
                (key.starts_with("toolset_perms:") && key.contains(id))
                    || key.starts_with("toolset_name:")
            })
        },
    );
    if let Err(error) = evicted {
        // LiteLLM's callers run this after the write already committed, so a failed eviction only warns
        tracing::warn!("invalidate_toolset_cache: failed to evict in-memory entries: {error}");
    }
}
