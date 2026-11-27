use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::lookups::{
    BudgetLookupError, BudgetLookups, UserLookupOptions,
};
use betterlite_logging::prometheus::budget_lookups::records::{
    CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord,
};
use serde_json::{Value, json};

use super::input::{BudgetRowCall, LookupCall};

// a BudgetLookups that answers from the case and writes down every read the assembly asked for
pub(super) struct ScriptedLookups {
    lookup: LookupCall,
    database: bool,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedLookups {
    pub(super) fn new(lookup: LookupCall, database: bool) -> Self {
        Self {
            lookup,
            database,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn note(&self, name: &str, args: Value) -> Option<&LookupCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({ "name": name, "args": args }));
        self.lookup.record()
    }
}

fn lookup_error(message: &str) -> BudgetLookupError {
    BudgetLookupError {
        message: message.to_owned(),
    }
}

fn snapshot(row: &BudgetRowCall) -> BudgetTableSnapshot {
    BudgetTableSnapshot {
        max_budget: row.max_budget,
        budget_reset_at: row.budget_reset_at,
    }
}

#[async_trait]
impl BudgetLookups for ScriptedLookups {
    fn database_configured(&self) -> bool {
        self.database
    }

    async fn team_info(&self, team_id: &str) -> Result<TeamRecord, BudgetLookupError> {
        let record = self
            .note("team_info", json!({ "team_id": team_id }))
            .ok_or_else(|| lookup_error("the team is not in the database"))?;
        Ok(TeamRecord {
            budget_reset_at: record.budget_reset_at,
            max_budget: record.max_budget,
        })
    }

    async fn org_info(
        &self,
        org_id: &str,
        include_budget_table: bool,
    ) -> Result<Option<OrgRecord>, BudgetLookupError> {
        let Some(record) = self.note(
            "org_info",
            json!({ "org_id": org_id, "include_budget_table": include_budget_table }),
        ) else {
            return Ok(None);
        };
        Ok(Some(OrgRecord {
            spend: record.spend,
            organization_alias: record.organization_alias.clone(),
            litellm_budget_table: record.budget.as_ref().map(snapshot),
        }))
    }

    async fn key_info(
        &self,
        hashed_token: &str,
        check_cache_only: bool,
    ) -> Result<KeyRecord, BudgetLookupError> {
        let record = self
            .note(
                "key_info",
                json!({ "hashed_token": hashed_token, "check_cache_only": check_cache_only }),
            )
            .ok_or_else(|| lookup_error("the key is not in the cache"))?;
        Ok(KeyRecord {
            budget_reset_at: record.budget_reset_at,
        })
    }

    async fn user_info(
        &self,
        user_id: &str,
        options: UserLookupOptions,
    ) -> Result<Option<UserRecord>, BudgetLookupError> {
        let Some(record) = self.note(
            "user_info",
            json!({
                "user_id": user_id,
                "user_id_upsert": options.user_id_upsert,
                "check_db_only": options.check_db_only,
            }),
        ) else {
            return Ok(None);
        };
        Ok(Some(UserRecord {
            budget_reset_at: record.budget_reset_at,
            max_budget: record.max_budget,
            user_email: record.user_email.clone(),
            user_alias: record.user_alias.clone(),
        }))
    }

    async fn cached_customer(
        &self,
        end_user_id: &str,
    ) -> Result<Option<CachedEndUser>, BudgetLookupError> {
        let cache_key = format!("end_user_id:{end_user_id}");
        let Some(record) = self.note("cached_customer", json!({ "cache_key": cache_key })) else {
            return Ok(None);
        };
        Ok(Some(CachedEndUser {
            spend: record.spend.unwrap_or_default(),
            litellm_budget_table: record.budget.as_ref().map(snapshot),
        }))
    }

    async fn default_customer_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<BudgetTableSnapshot>, BudgetLookupError> {
        let record = self.note("default_customer_budget", json!({ "budget_id": budget_id }));
        Ok(record.and_then(|record| record.budget.as_ref().map(snapshot)))
    }
}
