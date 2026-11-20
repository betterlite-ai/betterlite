use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::auth_pubsub::endpoint::AuthPubSub;
use betterlite_auth::auth_pubsub::eviction::LocalEviction;
use betterlite_auth::auth_pubsub::publish::PublishTasks;
use betterlite_auth::auth_pubsub::publish::evict_and_broadcast;

// ast-grep-ignore: no-long-names
pub async fn invalidate_cached_object_permissions<E>(
    object_permission_ids: &[Option<&str>],
    eviction: &dyn LocalEviction,
    tasks: &PublishTasks,
    endpoint: Option<&Arc<E>>,
) where
    E: AuthPubSub + 'static,
{
    let mut seen = BTreeSet::new();
    let mut cache_keys = Vec::new();
    for value in object_permission_ids {
        let Some(object_permission_id) = *value else {
            continue;
        };
        if !seen.insert(object_permission_id) {
            continue;
        }
        cache_keys.push(CacheKey::ObjectPermissionId(object_permission_id).render());
    }
    evict_and_broadcast(&cache_keys, eviction, tasks, endpoint).await;
}
