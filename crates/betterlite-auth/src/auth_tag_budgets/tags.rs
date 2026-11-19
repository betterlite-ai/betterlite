use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::tag::TagTable;

use super::TagBudgetError;
use super::cache::TagBudgetCache;
use super::merged;
use super::registry::load_tag_registry;
use super::rows::TagRows;
use crate::auth_cache::keys::CacheKey;

// LiteLLM's get_tag_objects_batch: probe every name, then one read for the misses the registry knows.
pub async fn get_tag_objects_batch(
    tag_names: &[String],
    database: bool,
    cache: &dyn TagBudgetCache,
    rows: &dyn TagRows,
) -> Result<BTreeMap<String, TagTable>, TagBudgetError> {
    if !database || tag_names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut probed = Vec::with_capacity(tag_names.len());
    for name in tag_names {
        let cached = cache
            .read_tag(&CacheKey::Tag(name).render())
            .await
            .map_err(|error| TagBudgetError::TagPayload(Box::new(error)))?;
        probed.push((name.clone(), cached));
    }
    let misses: Vec<String> = probed
        .iter()
        .filter(|(_, tag)| tag.is_none())
        .map(|(name, _)| name.clone())
        .collect();
    let fetched = fetch_uncached_tags(&misses, cache, rows).await?;
    Ok(merged(probed, fetched))
}

// LiteLLM's _fetch_uncached_tags: names a usable registry lacks never reach the database.
async fn fetch_uncached_tags(
    uncached_tags: &[String],
    cache: &dyn TagBudgetCache,
    rows: &dyn TagRows,
) -> Result<Vec<(String, TagTable)>, TagBudgetError> {
    if uncached_tags.is_empty() {
        return Ok(Vec::new());
    }
    let registry = load_tag_registry(cache, rows).await?;
    let tags_to_fetch: Vec<String> = match registry {
        Some(known) => uncached_tags
            .iter()
            .filter(|name| known.contains(name.as_str()))
            .cloned()
            .collect(),
        None => uncached_tags.to_vec(),
    };
    if tags_to_fetch.is_empty() {
        return Ok(Vec::new());
    }
    let db_tags = match rows.by_names(tags_to_fetch).await {
        Ok(found) => found,
        Err(failure) => {
            tracing::debug!(error = %failure, "Error batch fetching tags from database");
            return Ok(Vec::new());
        }
    };
    let ttl = cache.management_ttl();
    let mut fetched = Vec::with_capacity(db_tags.len());
    for tag in db_tags {
        let key = CacheKey::Tag(&tag.tag_name).render();
        if let Err(error) = cache.write_tag(&key, &tag, ttl).await {
            tracing::debug!(error = %error, "Error batch fetching tags from database");
            return Ok(Vec::new());
        }
        fetched.push((tag.tag_name.clone(), tag));
    }
    Ok(fetched)
}
