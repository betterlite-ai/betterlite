use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::types::ToSql;

use crate::app::spend::cli_writer_store;
use crate::shadow_result_aggregation::error::ShadowResultsError;
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::postgres::{PgShadowResults, leg_row};
use crate::shadow_result_aggregation::repository::RepositoryResult;
use crate::shadow_result_aggregation::targets::target_type_name;
use crate::state::ServerState;

use super::store::{ShadowLegReads, ShadowTargetFilter};

// _LIST_LEGS_SQL: the newest jobs in one bounded read; every leg of a selected job comes back
pub const LIST_LEGS_SQL: &str = r#"SELECT * FROM "LiteLLM_ShadowEvalJob"
WHERE "group_id" IN (
    SELECT "group_id" FROM "LiteLLM_ShadowEvalJob"
    GROUP BY "group_id" ORDER BY MAX("created_at") DESC LIMIT $1
)"#;

// _LIST_LEGS_BY_TARGET_SQL: the filter picks the jobs, never their legs
pub const LIST_LEGS_BY_TARGET_SQL: &str = r#"SELECT * FROM "LiteLLM_ShadowEvalJob"
WHERE "group_id" IN (
    SELECT "group_id" FROM "LiteLLM_ShadowEvalJob" WHERE "target_type" = $2 AND "target_id" = $3
    GROUP BY "group_id" ORDER BY MAX("created_at") DESC LIMIT $1
)"#;

pub const LEGS_OF_GROUP_SQL: &str =
    r#"SELECT * FROM "LiteLLM_ShadowEvalJob" WHERE "group_id" = $1"#;

#[async_trait]
impl ShadowLegReads for PgShadowResults {
    async fn list_legs(
        &self,
        limit: i64,
        target: Option<ShadowTargetFilter>,
    ) -> RepositoryResult<Vec<LegRow>> {
        match target {
            Some(filter) => {
                let kind = target_type_name(filter.target_type);
                leg_rows(
                    self,
                    LIST_LEGS_BY_TARGET_SQL,
                    &[&limit, &kind, &filter.target_id],
                )
                .await
            }
            None => leg_rows(self, LIST_LEGS_SQL, &[&limit]).await,
        }
    }

    async fn legs_of_group(&self, job_id: &str) -> RepositoryResult<Vec<LegRow>> {
        leg_rows(self, LEGS_OF_GROUP_SQL, &[&job_id]).await
    }
}

// a free function keeps PgShadowResults at one inherent impl, which the lint table allows
async fn leg_rows(
    results: &PgShadowResults,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> RepositoryResult<Vec<LegRow>> {
    let client = results.store.session().await?;
    let rows = client
        .query(sql, params)
        .await
        .map_err(|source| ShadowResultsError::Database(backend(source)))?;
    rows.iter().map(leg_row).collect()
}

// the route reads through the proxy's writer pool, the one the cache activity routes already open
#[must_use]
pub fn store_from_state(state: &Arc<ServerState>) -> Option<PgShadowResults> {
    cli_writer_store(state).map(|store| PgShadowResults {
        store: PostgresStore::clone(&store),
    })
}
