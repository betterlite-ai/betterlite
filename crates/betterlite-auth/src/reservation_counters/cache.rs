use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_tag_budgets::cache::TagBudgetCache;

// the typed rows the counter list reads, on top of the tag and group reads both batch lookups need
#[async_trait]
pub trait ReservationCounterCache: TagBudgetCache {
    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, PayloadError>;
    async fn read_organization(&self, key: &str)
    -> Result<Option<OrganizationTable>, PayloadError>;
    async fn read_membership(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, PayloadError>;
    async fn read_default_budget(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError>;
}

#[async_trait]
impl<T> ReservationCounterCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    async fn read_project(&self, key: &str) -> Result<Option<CachedProject>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn read_organization(
        &self,
        key: &str,
    ) -> Result<Option<OrganizationTable>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn read_membership(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn read_default_budget(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        self.async_get_typed(key, false).await
    }
}

// LiteLLM's async_get_cache turns an undecodable row into a miss, so the counter list drops that entity instead of failing
pub(crate) fn fail_open<Row>(
    outcome: Result<Option<Row>, PayloadError>,
    entity: &str,
) -> Option<Row> {
    match outcome {
        Ok(row) => row,
        Err(error) => {
            tracing::warn!(
                entity = %entity,
                error = %error,
                "Skipping a reservation counter: the cached row could not be read"
            );
            None
        }
    }
}
