use super::db::PrefetchDatabase;
use super::entries::{AuthObjectRefs, iter_entries};
use super::fill::{PrefetchClock, PrefetchError, fill_from_db, fill_from_redis, missing_in_memory};
use super::memory::PrefetchCache;
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::spend_counter_batch::scope::SpendCounterScope;

// B2316 binds the admission counters this request will read; the falsey end user is not bound
pub trait AdmissionCounterBinding: Send + Sync {
    fn bind_admission_counter_keys(&self, auth: &UserAPIKeyAuth, end_user_id: Option<&str>);
}
impl AdmissionCounterBinding for SpendCounterScope {
    fn bind_admission_counter_keys(&self, auth: &UserAPIKeyAuth, end_user_id: Option<&str>) {
        Self::bind_admission_counter_keys(self, auth, end_user_id);
    }
}
pub struct OwnerPrefetch<'context> {
    pub cache: &'context dyn PrefetchCache,
    pub database: Option<&'context dyn PrefetchDatabase>,
    pub clock: &'context dyn PrefetchClock,
    pub organization_ttl: f64,
}
impl OwnerPrefetch<'_> {
    // best effort: any failure leaves the per-object getters to fetch as before
    pub async fn prefetch(&self, refs: &AuthObjectRefs) {
        if let Err(error) = self.prefetch_inner(refs).await {
            tracing::warn!(
                %error,
                "auth prefetch skipped, falling back to per-object lookups"
            );
        }
    }
    async fn prefetch_inner(&self, refs: &AuthObjectRefs) -> Result<(), PrefetchError> {
        let all = iter_entries(refs, self.cache.management_ttl(), self.organization_ttl);
        let memory = self.cache.ordinary_memory();
        let missing = missing_in_memory(&all, memory);
        let redis = self.cache.redis();
        if let Some(redis) = redis.as_deref() {
            fill_from_redis(&missing, redis, memory).await?;
        }
        let Some(database) = self.database else {
            return Ok(());
        };
        let remaining = missing_in_memory(&missing, memory);
        fill_from_db(refs, &remaining, self.cache, database, self.clock).await
    }
}
// the request path binds counters before warming rows; the model authorization check runs earlier
pub struct ReferencedPrefetch<'context> {
    pub owner: OwnerPrefetch<'context>,
    pub admission: &'context dyn AdmissionCounterBinding,
}
impl ReferencedPrefetch<'_> {
    pub async fn prefetch(&self, auth: &UserAPIKeyAuth, end_user_id: Option<&str>) {
        let end_user_id = end_user_id.filter(|id| !id.is_empty());
        self.admission
            .bind_admission_counter_keys(auth, end_user_id);
        self.owner
            .prefetch(&AuthObjectRefs::from_token(&auth.view.token))
            .await;
    }
}
