use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{
    BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure,
};
use betterlite_auth::budget_primitives::carried::carried_metadata;
use betterlite_auth::budget_snapshots::BudgetSnapshotError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::{Value, json};

// the tag and access-group rows a case seeds, behind one cache that never hits
pub(crate) struct TagWorld {
    tags: BTreeMap<String, TagTable>,
    groups: BTreeMap<String, ModelAccessGroupBudgetRow>,
}

impl TagWorld {
    pub(crate) fn new(tags: Vec<TagTable>, groups: Vec<ModelAccessGroupBudget>) -> Self {
        Self {
            tags: tags
                .into_iter()
                .map(|row| (row.tag_name.clone(), row))
                .collect(),
            groups: groups
                .into_iter()
                .map(|budget| (budget.access_group_name.clone(), group_row(budget)))
                .collect(),
        }
    }
}

// the flattened cache payload a case hands over, rebuilt as the row the batch read projects
fn group_row(budget: ModelAccessGroupBudget) -> ModelAccessGroupBudgetRow {
    ModelAccessGroupBudgetRow {
        access_group_name: budget.access_group_name,
        spend: Some(budget.spend),
        litellm_budget_table: budget.max_budget.map(max_budget_table),
    }
}

fn max_budget_table(max_budget: Decimal) -> LiteLLMBudgetTable {
    LiteLLMBudgetTable {
        budget_id: None,
        soft_budget: None,
        max_budget: Some(max_budget),
        max_parallel_requests: None,
        tpm_limit: None,
        rpm_limit: None,
        tpd_limit: None,
        model_max_budget: None,
        budget_duration: None,
        allowed_models: None,
        temp_budget_increase: None,
        temp_budget_expiry: None,
    }
}

#[async_trait]
impl TagBudgetCache for TagWorld {
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
impl BudgetRows<TagTable> for TagWorld {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self
            .tags
            .values()
            .filter(|row| names.contains(&row.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self.tags.values().cloned().collect())
    }
}

#[async_trait]
impl BudgetRows<ModelAccessGroupBudgetRow> for TagWorld {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(self
            .groups
            .values()
            .filter(|row| names.contains(&row.access_group_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(self.groups.values().cloned().collect())
    }
}

// the organization lookup a case seeds, either answering its one row or failing like an unreachable database
pub(crate) struct OrgWorld {
    organization: Option<OrganizationTable>,
    raises: bool,
}

pub(crate) fn org_world(organization: Option<OrganizationTable>, raises: bool) -> Arc<OrgWorld> {
    Arc::new(OrgWorld {
        organization,
        raises,
    })
}

pub(crate) fn org_lookup(world: &Arc<OrgWorld>) -> OrgLookup {
    let cache: Arc<dyn OrgCache> = Arc::<OrgWorld>::clone(world);
    let store: Arc<dyn OrgStore> = Arc::<OrgWorld>::clone(world);
    OrgLookup {
        store: Some(store),
        cache,
        in_memory_ttl: 5.0_f64,
    }
}

#[async_trait]
impl OrgCache for OrgWorld {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> Result<Option<OrganizationTable>, AuthCacheError> {
        Ok(None)
    }

    async fn write_org(
        &self,
        _key: &str,
        _org: &OrganizationTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> Result<Option<CachedProject>, AuthCacheError> {
        Ok(None)
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl OrgStore for OrgWorld {
    async fn find_org(&self, org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        if self.raises {
            return Err(Box::new(AuthReadError::Repository(DbError::CreateNull)));
        }
        let found = self
            .organization
            .clone()
            .filter(|row| row.organization_id.as_deref() == Some(org_id));
        Ok(found
            .map(|row| serde_json::to_string(&row).expect("the seeded organization serializes")))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}

// organization_alias beside the OrgBudgetSnapshot metadata entries the request carries
pub(crate) fn carried_record(token: &UserAPIKeyAuth) -> Result<Value, CarriedError> {
    let entries = carried_metadata(token)?;
    let Some(spend) = entries.get("user_api_key_org_spend") else {
        return Ok(Value::Null);
    };
    let max_budget = match entries.get("user_api_key_org_max_budget") {
        Some(raw) => serde_json::from_str::<Value>(raw.get())?,
        None => Value::Null,
    };
    let encoded = serde_json::to_value(token)?;
    Ok(json!({
        "organization_alias": encoded.get("organization_alias").cloned().unwrap_or(Value::Null),
        "user_api_key_org_spend": serde_json::from_str::<Value>(spend.get())?,
        "user_api_key_org_max_budget": max_budget,
    }))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CarriedError {
    #[error(transparent)]
    Snapshot(#[from] BudgetSnapshotError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
