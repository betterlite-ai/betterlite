use std::future::Future;

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use thiserror::Error;
use tokio_postgres::Row;

use super::cache::{AccessGroupCache, invalidate_access_group_caches};

pub mod postgres;

// team_endpoints reuses this exact statement to serialize member writes against this mirror
pub const TEAM_ADVISORY_LOCK_SQL: &str =
    "SELECT pg_advisory_xact_lock(hashtext($1)) IS NULL AS locked";

#[derive(Debug, Error)]
pub enum TeamSyncError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Cache(#[from] InvalidationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedGroupRow {
    pub access_group_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamGroupRow {
    pub access_group_ids: Option<Vec<String>>,
}

impl AffectedGroupRow {
    fn from_row(row: &Row) -> Result<Self, TeamSyncError> {
        let access_group_id = row
            .try_get(0)
            .map_err(|source| TeamSyncError::Database(backend(source)))?;
        Ok(Self { access_group_id })
    }
}

impl TeamGroupRow {
    fn from_row(row: &Row) -> Result<Self, TeamSyncError> {
        let access_group_ids = row
            .try_get(0)
            .map_err(|source| TeamSyncError::Database(backend(source)))?;
        Ok(Self { access_group_ids })
    }
}

pub trait TeamSyncTransaction: Send + Sync {
    fn lock_team(&self, team_id: &str) -> impl Future<Output = Result<(), TeamSyncError>> + Send;

    fn team_groups(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<TeamGroupRow>, TeamSyncError>> + Send;

    fn affected_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<Vec<AffectedGroupRow>, TeamSyncError>> + Send;

    fn attach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send;

    fn detach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send;
}

pub async fn reconcile_team_access_groups<Tx>(
    transaction: &Tx,
    team_id: &str,
) -> Result<Vec<String>, TeamSyncError>
where
    Tx: TeamSyncTransaction + ?Sized,
{
    transaction.lock_team(team_id).await?;
    let desired = transaction
        .team_groups(team_id)
        .await?
        .and_then(|row| row.access_group_ids)
        .unwrap_or_default();
    let affected = transaction.affected_groups(team_id, &desired).await?;
    transaction.attach_groups(team_id, &desired).await?;
    transaction.detach_groups(team_id, &desired).await?;
    Ok(affected
        .into_iter()
        .map(|row| row.access_group_id)
        .collect())
}

pub async fn sync_team_access_groups<Cache>(
    store: &PostgresStore,
    cache: &Cache,
    team_id: &str,
) -> Result<(), TeamSyncError>
where
    Cache: AccessGroupCache + ?Sized,
{
    let mut client = store.session().await?;
    let transaction = client
        .transaction()
        .await
        .map_err(|source| TeamSyncError::Database(backend(source)))?;
    let affected = reconcile_team_access_groups(&transaction, team_id).await?;
    transaction
        .commit()
        .await
        .map_err(|source| TeamSyncError::Database(backend(source)))?;
    invalidate_access_group_caches(cache, &affected).await?;
    Ok(())
}
