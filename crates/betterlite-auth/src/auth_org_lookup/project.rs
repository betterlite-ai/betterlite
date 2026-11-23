use crate::auth_prefetch::rows::CachedProject;

use super::error::{OrgFailure, OrgLookupError};
use super::organization::OrgLookup;

// get_project_object: no DB is "no project" before the cache, and a cache hit never refreshes the row
pub async fn get_project_object(
    lookup: &OrgLookup,
    project_id: &str,
    now: f64,
) -> Result<Option<CachedProject>, OrgLookupError> {
    let Some(store) = lookup.store.as_deref() else {
        return Ok(None);
    };
    let key = format!("project_id:{project_id}");
    let cached = lookup
        .cache
        .read_project(&key)
        .await
        .map_err(|error| OrgLookupError::CacheRead(Box::new(error)))?;
    if let Some(project) = cached {
        return Ok(Some(project));
    }
    let found = store
        .find_project(project_id)
        .await
        .map_err(OrgFailure::Store)?;
    let Some(text) = found else {
        return Ok(None);
    };
    let mut project = serde_json::from_str::<CachedProject>(&text)
        .map_err(|source| OrgFailure::Row(Box::new(source)))?;
    project.last_refreshed_at = Some(now);
    lookup
        .cache
        .write_project(&key, &project, lookup.cache.management_ttl())
        .await
        .map_err(|error| OrgFailure::Cache(Box::new(error)))?;
    Ok(Some(project))
}
