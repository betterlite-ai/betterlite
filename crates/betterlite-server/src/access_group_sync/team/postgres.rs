use std::future::Future;

use betterlite_db::postgres_store::errors::backend;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{
    AffectedGroupRow, TEAM_ADVISORY_LOCK_SQL, TeamGroupRow, TeamSyncError, TeamSyncTransaction,
};

const READ_TEAM_SQL: &str = "SELECT access_group_ids FROM \"LiteLLM_TeamTable\" WHERE team_id = $1";
const AFFECTED_SQL: &str = "SELECT access_group_id FROM \"LiteLLM_AccessGroupTable\" WHERE access_group_id = ANY($2::TEXT[]) OR $1 = ANY(COALESCE(assigned_team_ids, ARRAY[]::TEXT[]))";
const ATTACH_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET assigned_team_ids = array_append(COALESCE(assigned_team_ids, ARRAY[]::TEXT[]), $1) WHERE access_group_id = ANY($2::TEXT[]) AND NOT ($1 = ANY(COALESCE(assigned_team_ids, ARRAY[]::TEXT[]))) RETURNING access_group_id";
const DETACH_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET assigned_team_ids = array_remove(assigned_team_ids, $1) WHERE $1 = ANY(COALESCE(assigned_team_ids, ARRAY[]::TEXT[])) AND NOT (access_group_id = ANY($2::TEXT[])) RETURNING access_group_id";

impl TeamSyncTransaction for deadpool_postgres::Transaction<'_> {
    fn lock_team(&self, team_id: &str) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        async move {
            run(self, TEAM_ADVISORY_LOCK_SQL, &[&team_id])
                .await
                .map(drop)
        }
    }

    fn team_groups(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<TeamGroupRow>, TeamSyncError>> + Send {
        async move { team_row(self, team_id).await }
    }

    fn affected_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<Vec<AffectedGroupRow>, TeamSyncError>> + Send {
        async move { affected_rows(self, team_id, desired).await }
    }

    fn attach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        async move { run(self, ATTACH_SQL, &[&team_id, &desired]).await.map(drop) }
    }

    fn detach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        async move { run(self, DETACH_SQL, &[&team_id, &desired]).await.map(drop) }
    }
}

async fn team_row(
    transaction: &deadpool_postgres::Transaction<'_>,
    team_id: &str,
) -> Result<Option<TeamGroupRow>, TeamSyncError> {
    let rows = run(transaction, READ_TEAM_SQL, &[&team_id]).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(TeamGroupRow::from_row(row)?))
}

async fn affected_rows(
    transaction: &deadpool_postgres::Transaction<'_>,
    team_id: &str,
    desired: &[String],
) -> Result<Vec<AffectedGroupRow>, TeamSyncError> {
    run(transaction, AFFECTED_SQL, &[&team_id, &desired])
        .await?
        .iter()
        .map(AffectedGroupRow::from_row)
        .collect()
}

async fn run(
    transaction: &deadpool_postgres::Transaction<'_>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, TeamSyncError> {
    transaction
        .query(sql, parameters)
        .await
        .map_err(|source| TeamSyncError::Database(backend(source)))
}
