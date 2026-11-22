use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use super::error::VisibilityError;
use super::owners::OwnerPair;
use super::query::PayloadQuery;

// the columns _spend_log_payload_query selects; None is a SQL NULL column
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRow {
    pub request_id: String,
    pub messages: Option<JsonData>,
    pub response: Option<JsonData>,
    pub proxy_server_request: Option<JsonData>,
    pub metadata: Option<JsonData>,
    pub user: Option<String>,
    pub team_id: Option<String>,
}

// the four reads the visibility checks make, each at the moment the check runs
#[async_trait]
pub trait VisibilityStore: Send + Sync {
    async fn owners(&self, lookup_id: &str) -> Result<Vec<OwnerPair>, VisibilityError>;
    async fn payload_rows(
        &self,
        query: &PayloadQuery<'_>,
    ) -> Result<Vec<PayloadRow>, VisibilityError>;
    // _find_team_row: the team table itself, never the cached team object
    async fn team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, VisibilityError>;
    // _find_team_rows: the rows in the order the database returns them
    async fn teams(&self, team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError>;
}
