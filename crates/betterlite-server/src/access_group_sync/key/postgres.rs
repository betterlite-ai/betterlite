use betterlite_db::auxiliary_repository::bindings::AccessGroupTable;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{KeyMembershipDatabase, KeyMembershipStatement, KeySyncError, MovedGroupRow};

const ATTACH_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_append(\"assigned_key_ids\", $1) WHERE \"access_group_id\" = ANY($2::text[]) AND NOT ($1 = ANY(\"assigned_key_ids\")) RETURNING \"access_group_id\"";
const DETACH_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_remove(\"assigned_key_ids\", $1) WHERE \"access_group_id\" = ANY($2::text[]) AND $1 = ANY(\"assigned_key_ids\") RETURNING \"access_group_id\"";
const REPOINT_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_append(array_remove(array_remove(\"assigned_key_ids\", $1), $2), $2) WHERE $1 = ANY(\"assigned_key_ids\") RETURNING \"access_group_id\"";

impl KeyMembershipStatement {
    const fn sql(self) -> &'static str {
        match self {
            Self::Attach => ATTACH_KEY_SQL,
            Self::Detach => DETACH_KEY_SQL,
        }
    }
}

pub struct PostgresKeyMembership<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl KeyMembershipDatabase for PostgresKeyMembership<'_> {
    async fn move_key_membership(
        &self,
        statement: KeyMembershipStatement,
        key_token: &str,
        access_group_ids: &[String],
    ) -> Result<Vec<MovedGroupRow>, KeySyncError> {
        let parameters: [&(dyn ToSql + Sync); 2] = [&key_token, &access_group_ids];
        moved_groups(self.client, statement.sql(), &parameters).await
    }

    async fn repoint_key_membership(
        &self,
        previous: &str,
        new: &str,
    ) -> Result<Vec<MovedGroupRow>, KeySyncError> {
        moved_groups(self.client, REPOINT_KEY_SQL, &[&previous, &new]).await
    }
}

impl MovedGroupRow {
    fn from_row(row: &Row) -> Result<Self, KeySyncError> {
        let access_group_id = row
            .try_get(0)
            .map_err(|source| KeySyncError::Database(backend(source)))?;
        Ok(Self { access_group_id })
    }
}

async fn moved_groups(
    client: Option<PrismaClient<'_>>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<MovedGroupRow>, KeySyncError> {
    let repository = PrismaTableRepository::new(
        client,
        redis_config_change_publisher(None),
        AccessGroupTable,
    );
    let resolved = repository.prisma_client()?;
    // B714 pins raw SQL writes to the writer handle even while the writer reports degraded
    let pinned = WriterPinnedClient::Writer(resolved.writer);
    let store = writer_wrapper(pinned);
    let session = store.session().await?;
    let rows = session
        .query(sql, parameters)
        .await
        .map_err(|source| KeySyncError::Database(backend(source)))?;
    rows.iter().map(MovedGroupRow::from_row).collect()
}
