use std::collections::BTreeSet;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_request_models::catalog::{
    ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
};
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{
    BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure,
};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::reservation_admission::engine::{Admission, AdmissionRequest};
use betterlite_auth::reservation_admission::error::AdmissionError;
use betterlite_auth::reservation_cost::EstimateRequest;
use betterlite_auth::reservation_cost::error::ReservationCostError;
use betterlite_auth::reservation_counters::cache::ReservationCounterCache;
use betterlite_auth::reservation_request::ports::{
    ReservationAdmission, ReservationCostSource, ReservationSettlement,
};
use betterlite_auth::reservation_settlement::counters::ScopeHold;
use betterlite_auth::reservation_settlement::entry::ReservationEntry;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

// the helper runs with prisma_client=None, so every entity row read misses
pub(crate) struct NoCache;

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

// with no token tags and no access groups the batch lookups ask for nothing
pub(crate) struct NoTags;

#[async_trait]
impl BudgetRows<TagTable> for NoTags {
    async fn by_names(&self, _names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(Vec::new())
    }
}

pub(crate) struct NoGroups;

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
pub(crate) struct Durations(pub(crate) TimeZoneDatabase);

impl DurationParser for Durations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|duration| duration.seconds)
    }
}

#[derive(Debug)]
pub(crate) struct NoDecoders;

impl ManagedIdDecoders for NoDecoders {
    fn decode_family(
        &self,
        _family: ManagedIdFamily,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

// B3001's counts recorded, its costs absent: the case observes whether token counting was reached
#[derive(Default)]
pub(crate) struct RecordingCosts {
    pub(crate) counted: Mutex<bool>,
}

#[async_trait]
impl ReservationCostSource for RecordingCosts {
    async fn input_tokens(
        &self,
        _request: &EstimateRequest<'_>,
    ) -> Result<IndexMap<String, usize>, ReservationCostError> {
        *lock(&self.counted) = true;
        Ok(IndexMap::new())
    }

    async fn max_cost(
        &self,
        _request: &EstimateRequest<'_>,
        _input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        Ok(None)
    }

    async fn input_cost(
        &self,
        _request: &EstimateRequest<'_>,
        _input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        Ok(None)
    }
}

// the case stops before admission, so a call here is a bug the helper reports as an error
pub(crate) struct NeverAdmission;

#[async_trait]
impl ReservationAdmission for NeverAdmission {
    async fn admit(
        &self,
        _request: &AdmissionRequest<'_>,
        _applied_entries: &mut Vec<ReservationEntry>,
    ) -> Result<Admission, AdmissionError> {
        Err(AdmissionError::Overflow)
    }
}

pub(crate) struct NeverSettlement;

struct NoScope;

impl ScopeHold for NoScope {}

#[async_trait]
impl ReservationSettlement for NeverSettlement {
    fn enter_scope(&self, _counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_> {
        Box::new(NoScope)
    }

    async fn release_entries(
        &self,
        _entries: &mut [ReservationEntry],
        _default_reserved_cost: Decimal,
    ) {
    }
}
