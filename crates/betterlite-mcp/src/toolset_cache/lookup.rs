use betterlite_config::sections::mcp_toolset_storage::ToolsetLookup;

use super::{CachedToolset, ToolsetByName, ToolsetCache};

pub async fn toolset_by_name_cached<C, S>(cache: &C, store: &S, toolset_name: &str) -> ToolsetLookup
where
    C: ToolsetCache,
    S: ToolsetByName,
{
    let key = format!("toolset_name:{toolset_name}");
    if let Some(cached) = cache.get(&key).await {
        return match cached {
            CachedToolset::Found(toolset) => Ok(Some(toolset)),
            CachedToolset::NotFound => Ok(None),
        };
    }
    let found = store.by_name(toolset_name).await?;
    let cached = found.as_ref().map_or(CachedToolset::NotFound, |toolset| {
        CachedToolset::Found(toolset.clone())
    });
    cache
        .set(&key, &cached, cache.management_object_ttl())
        .await;
    Ok(found)
}
