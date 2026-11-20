use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, TagBudgetRowsFailure};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;
use betterlite_rate_limit::identity_descriptors::agent_registry::AgentPolicyRegistry;
use betterlite_rate_limit::limiter_records::{TagRateLimit, TagRateLimitResolver};

pub(super) struct MapResolver {
    limits: BTreeMap<String, TagRateLimit>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl MapResolver {
    pub(super) fn new(limits: BTreeMap<String, TagRateLimit>) -> Self {
        Self {
            limits,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Vec<String>> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl TagRateLimitResolver for MapResolver {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        match self.calls.lock() {
            Ok(mut calls) => calls.push(tag_names.to_vec()),
            Err(poisoned) => poisoned.into_inner().push(tag_names.to_vec()),
        }
        self.limits.clone()
    }
}

pub(super) struct TokenCache(pub(super) BTreeMap<String, TagTable>);

#[async_trait]
impl TagBudgetCache for TokenCache {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(self.0.get(key).cloned())
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

pub(super) struct FixtureRows(pub(super) Vec<TagTable>);

#[async_trait]
impl BudgetRows<TagTable> for FixtureRows {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self
            .0
            .iter()
            .filter(|tag| names.contains(&tag.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self.0.clone())
    }
}

pub(super) struct FixedRegistry(pub(super) BTreeMap<String, AgentResponse>);

impl AgentPolicyRegistry for FixedRegistry {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0.get(agent_id).cloned()
    }
}
