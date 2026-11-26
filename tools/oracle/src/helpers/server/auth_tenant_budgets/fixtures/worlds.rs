use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
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

// the rows a batch read answers with; both maps are keyed by the requested name
fn rows_named<Row: Clone>(rows: &BTreeMap<String, Row>, names: &[String]) -> Vec<Row> {
    names
        .iter()
        .filter_map(|name| rows.get(name))
        .cloned()
        .collect()
}

#[async_trait]
impl BudgetRows<TagTable> for TagWorld {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(rows_named(&self.tags, &names))
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
        Ok(rows_named(&self.groups, &names))
    }

    async fn registry_rows(
        &self,
        _take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Ok(self.groups.values().cloned().collect())
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
