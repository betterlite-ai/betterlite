use betterlite_db::resource_rows::access_group::AccessGroupTable;

use crate::auth_team_lookup::error::TeamLookupError;

// The server backs this with the agent read-through (B8645) and B737's get_access_object
pub trait AccessGroupReads: Sync {
    fn has_database(&self) -> bool;

    // empty for an unknown agent: LiteLLM answers a failed resync as if the agent was missing
    fn agent_access_group_ids(&self, agent_id: &str) -> impl Future<Output = Vec<String>> + Send;

    // check_db_only skips the cache and reads the writer
    fn access_group(
        &self,
        access_group_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<AccessGroupTable, TeamLookupError>> + Send;
}
