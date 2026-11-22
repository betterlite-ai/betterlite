use std::future::Future;

use betterlite_db::postgres_store::errors::DbError;

// LiteLLM's AgentsRepository table read: find_many(where={agent_access_groups: {hasSome: groups}})
pub trait AgentTableReads: Sync {
    // LiteLLM's `prisma_client is None`: no table means no rows, not a failed read
    fn has_database(&self) -> bool;

    // the agent_id of every row whose agent_access_groups names one of the groups
    fn agent_ids_with_access_groups(
        &self,
        groups: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<Vec<String>, DbError>> + Send;
}
