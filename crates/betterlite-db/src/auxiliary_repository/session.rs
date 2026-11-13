use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient;

use crate::managed_resource_rows::autorouter::AutoRouterSession;
use crate::postgres_store::errors::backend;

use super::repository::{PrismaClient, PrismaTableRepository, TableBinding, TableError};

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

    pub async fn find_latest_for_key(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, TableError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(LATEST_SQL, &[&api_key, &session_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(TableError::Row)
    }
}
