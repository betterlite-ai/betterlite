use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::tag::TagTable;

use super::TagBudgetError;
use super::cache::{CachedValue, TagBudgetCache};
use super::merged;
use super::registry::{RegistryKind, load_registry};
use super::rows::BudgetRows;
use crate::auth_cache::keys::CacheKey;

// LiteLLM's get_tag_objects_batch: probe every name, then one read for the misses the registry knows.
pub async fn get_tag_objects_batch(
    tag_names: &[String],
    database: bool,
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<TagTable>,
) -> Result<BTreeMap<String, TagTable>, TagBudgetError> {
    if !database || tag_names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut probed = Vec::with_capacity(tag_names.len());
    let mut misses = Vec::new();
    for name in tag_names {
        let cached = cache
            .read_tag(&CacheKey::Tag(name).render())
            .await
            .map_err(|error| TagBudgetError::TagPayload(Box::new(error)))?;
        if cached.is_none() {
            misses.push(name.clone());
        }
        probed.push((name.clone(), cached));
    }
    let fetched = fetch_missing_tags(&misses, cache, rows).await?;
    Ok(merged(probed, fetched))
}

// LiteLLM's _fetch_uncached_tags: names a usable registry lacks never reach the database.
async fn fetch_missing_tags(
    uncached_tags: &[String],
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<TagTable>,
) -> Result<Vec<(String, TagTable)>, TagBudgetError> {
    if uncached_tags.is_empty() {
        return Ok(Vec::new());
    }
    let known = load_registry(RegistryKind::Tag, cache, rows).await?;
    let tags_to_fetch: Vec<String> = uncached_tags
        .iter()
        .filter(|name| {
            known
                .as_ref()
                .is_none_or(|available| available.contains(name.as_str()))
        })
        .cloned()
        .collect();
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
        if let Err(error) = cache.write(&key, CachedValue::Tag(&tag), ttl).await {
            tracing::debug!(error = %error, "Error batch fetching tags from database");
            return Ok(Vec::new());
        }
        fetched.push((tag.tag_name.clone(), tag));
    }
    Ok(fetched)
}
