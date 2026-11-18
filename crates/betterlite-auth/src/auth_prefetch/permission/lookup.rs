use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;

use super::cache::ObjectPermissionCache;
use super::error::PermissionError;
use super::store::ObjectPermissionStore;
use crate::auth_cache::keys::CacheKey;

// the normal and writer routes LiteLLM builds its repository from for this call
#[derive(Clone, Copy)]
pub struct PermissionStores<'source> {
    pub normal: &'source dyn ObjectPermissionStore,
    pub writer: &'source dyn ObjectPermissionStore,
}

// a permission lookup needs a cache and, for the database part, both routed stores
pub struct ObjectPermissionSource<'source> {
    pub cache: &'source dyn ObjectPermissionCache,
    pub stores: Option<PermissionStores<'source>>,
}

// auth_checks.py get_object_permission: cache first, then one keyed database read, then backfill
pub async fn get_object_permission(
    source: &ObjectPermissionSource<'_>,
    object_permission_id: &str,
    check_db_only: bool,
) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
    let Some(stores) = source.stores else {
        return Err(PermissionError::NoDatabase);
    };
    let key = CacheKey::ObjectPermissionId(object_permission_id).render();
    if !check_db_only {
        let cached = source
            .cache
            .get(&key)
            .await
            .map_err(PermissionError::Cache)?;
        if let Some(permission) = cached {
            return Ok(Some(permission));
        }
    }
    let outcome = lookup_permission(
        source.cache,
        stores,
        &key,
        object_permission_id,
        check_db_only,
    )
    .await;
    match outcome {
        Ok(permission) => Ok(permission),
        Err(error) if check_db_only => Err(error),
        Err(_swallowed) => Ok(None),
    }
}

async fn lookup_permission(
    cache: &dyn ObjectPermissionCache,
    stores: PermissionStores<'_>,
    key: &str,
    object_permission_id: &str,
    check_db_only: bool,
) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
    let store = if check_db_only {
        stores.writer
    } else {
        stores.normal
    };
    let Some(row) = store.find_permission(object_permission_id).await? else {
        return if check_db_only {
            Err(PermissionError::MissingInDatabaseOnly)
        } else {
            Ok(None)
        };
    };
    let wire = serde_json::to_value(row).map_err(PermissionError::Validation)?;
    let permission = serde_json::from_value::<LiteLLMObjectPermissionTable>(wire)
        .map_err(PermissionError::Validation)?;
    let ttl = cache.management_ttl();
    cache
        .set(key, &permission, ttl)
        .await
        .map_err(PermissionError::Cache)?;
    Ok(Some(permission))
}
