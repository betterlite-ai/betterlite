use std::future::Future;

use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use futures_util::future::join_all;

pub trait AccessGroupCache: Send + Sync {
    fn delete_access_group(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<(), InvalidationError>> + Send;
}

impl AccessGroupCache for AuthInvalidation {
    fn delete_access_group(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<(), InvalidationError>> + Send {
        self.delete_cache_access_object(access_group_id)
    }
}

// LiteLLM runs every eviction and only then raises the first failure in input order, keeping duplicates
pub async fn invalidate_access_group_caches<Cache>(
    cache: &Cache,
    access_group_ids: &[String],
) -> Result<(), InvalidationError>
where
    Cache: AccessGroupCache + ?Sized,
{
    let attempts = access_group_ids
        .iter()
        .map(|access_group_id| cache.delete_access_group(access_group_id));
    for outcome in join_all(attempts).await {
        outcome?;
    }
    Ok(())
}
