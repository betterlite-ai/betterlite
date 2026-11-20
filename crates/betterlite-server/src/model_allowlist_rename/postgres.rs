use betterlite_db::auxiliary_repository::bindings::TeamTable;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::statement::{AllowlistRewrite, TouchedRow};
use super::{AllowlistDatabase, ModelAllowlistError};

// The statement spans five tables, so the repository binding only resolves the writer B714 pins.
pub struct PostgresAllowlists<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl AllowlistDatabase for PostgresAllowlists<'_> {
    async fn rewrite_allowlists(
        &self,
        rewrite: &AllowlistRewrite,
    ) -> Result<Vec<TouchedRow>, ModelAllowlistError> {
        let parameters: [&(dyn ToSql + Sync); 2] = match rewrite {
            AllowlistRewrite::Append { old_name, new_name }
            | AllowlistRewrite::Replace { old_name, new_name } => [old_name, new_name],
        };
        let sql = rewrite.sql();
        let rows = writer_rows(self.client, &sql, &parameters).await?;
        rows.iter().map(TouchedRow::from_row).collect()
    }
}

impl TouchedRow {
    fn from_row(row: &Row) -> Result<Self, ModelAllowlistError> {
        let kind: String = row
            .try_get("kind")
            .map_err(|source| ModelAllowlistError::Database(backend(source)))?;
        let object_id: String = row
            .try_get("object_id")
            .map_err(|source| ModelAllowlistError::Database(backend(source)))?;
        let team_alias: Option<String> = row
            .try_get("team_alias")
            .map_err(|source| ModelAllowlistError::Database(backend(source)))?;
        Self::from_values(&kind, object_id, team_alias)
    }
}

async fn writer_rows(
    client: Option<PrismaClient<'_>>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, ModelAllowlistError> {
    let repository =
        PrismaTableRepository::new(client, redis_config_change_publisher(None), TeamTable);
    let resolved = repository.prisma_client()?;
    let pinned = WriterPinnedClient::Writer(resolved.writer);
    let store = writer_wrapper(pinned);
    let session = store.session().await?;
    session
        .query(sql, parameters)
        .await
        .map_err(|source| ModelAllowlistError::Database(backend(source)))
}
