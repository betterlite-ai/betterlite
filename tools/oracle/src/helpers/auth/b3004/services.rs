use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{
    BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure,
};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::reservation_counters::cache::ReservationCounterCache;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

// LiteLLM runs the helper with prisma_client=None, so every entity row read misses
pub(super) struct NoCache;

#[async_trait]
impl TagBudgetCache for NoCache {
    async fn read_tag(&self, _key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(None)
    }

    async fn read_group(&self, _key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(None)
    }

    async fn read_registry(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl ReservationCounterCache for NoCache {
    async fn read_project(&self, _key: &str) -> Result<Option<CachedProject>, PayloadError> {
        Ok(None)
    }

    async fn read_organization(
        &self,
        _key: &str,
    ) -> Result<Option<OrganizationTable>, PayloadError> {
        Ok(None)
    }

    async fn read_membership(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, PayloadError> {
        Ok(None)
    }

    async fn read_default_budget(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        Ok(None)
    }
}

// with no token tags the batch lookup asks for nothing, so the table behind it can stay empty
pub(super) struct NoTags;

#[async_trait]
impl BudgetRows<TagTable> for NoTags {
    async fn by_names(&self, _names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }
}

pub(super) struct NoGroups;

#[async_trait]
impl BudgetRows<ModelAccessGroupBudgetRow> for NoGroups {
    async fn by_names(
        &self,
        _names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }
}

// litellm's duration_in_seconds: a budget window's length in seconds, read with the same parser
pub(super) struct Durations(pub(super) TimeZoneDatabase);

impl DurationParser for Durations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|duration| duration.seconds)
    }
}
