use betterlite_redis::config_sync::publish::PublishChange;

use crate::managed_resource_rows::autorouter::AutoRouterSession;

use super::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError, query_json,
};

// Postgres orders DESC with NULLs first; no NULLS LAST or tie-breaker is added
const LATEST_SQL: &str = "SELECT to_jsonb(session_row)::text FROM \"LiteLLM_AutoRouterSession\" AS session_row \
     WHERE session_row.api_key = $1 AND session_row.session_id = $2 \
     ORDER BY session_row.last_turn_at DESC LIMIT 1";

pub struct AutoRouterSessionTable;

impl TableBinding for AutoRouterSessionTable {
    const NAME: &'static str = "litellm_autoroutersession";
    const TABLE: &'static str = "LiteLLM_AutoRouterSession";
}

pub struct AutoRouterSessionRepository<'db> {
    base: PrismaTableRepository<'db, AutoRouterSessionTable>,
}

impl<'db> AutoRouterSessionRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, AutoRouterSessionTable),
        }
    }

    // ast-grep-ignore: no-padded-one-line-functions - LiteLLM's one-query public read
    pub async fn find_latest_for_key(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, TableError> {
        let access = self.base.access()?;
        query_json(access.read, LATEST_SQL, &[&api_key, &session_id]).await
    }
}
