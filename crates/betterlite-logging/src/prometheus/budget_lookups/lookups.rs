use async_trait::async_trait;

use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;

use super::records::{CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord};

// LiteLLM catches whatever a lookup raises and writes its text to a debug line, so the failure carries a message
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct BudgetLookupError {
    pub message: String,
}

// LiteLLM's get_user_object takes these two flags, and threading them as a struct keeps the boolean count down
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserLookupOptions {
    pub user_id_upsert: bool,
    pub check_db_only: bool,
}

// The cache and database reads LiteLLM's assembly calls make, one method each:
// get_team_object, get_org_object, get_key_object, get_user_object, the end-user cache read
// and the default-budget table read. The implementation lives where those clients do.
#[async_trait]
pub trait BudgetLookups: Send + Sync {
    fn database_configured(&self) -> bool;

    async fn team_info(&self, team_id: &str) -> Result<TeamRecord, BudgetLookupError>;

    async fn org_info(
        &self,
        org_id: &str,
        include_budget_table: bool,
    ) -> Result<Option<OrgRecord>, BudgetLookupError>;

    async fn key_info(
        &self,
        hashed_token: &str,
        check_cache_only: bool,
    ) -> Result<KeyRecord, BudgetLookupError>;

    async fn user_info(
        &self,
        user_id: &str,
        options: UserLookupOptions,
    ) -> Result<Option<UserRecord>, BudgetLookupError>;

    // the implementation renders the end_user_cache_key literal from the id, since the cache protocol key belongs to the cache crate
    async fn cached_customer(
        &self,
        end_user_id: &str,
    ) -> Result<Option<CachedEndUser>, BudgetLookupError>;

    async fn default_customer_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<BudgetTableSnapshot>, BudgetLookupError>;
}
