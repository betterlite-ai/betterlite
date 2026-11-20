use std::convert::Infallible;
use std::future::Future;
use std::time::Duration;

use betterlite_db::auxiliary_repository::bindings::AccessGroupTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding,
};
use betterlite_db::database_refresh::handle::{PoolFuture, PoolTransport};
use betterlite_db::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use betterlite_db::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use deadpool_postgres::Pool;
use secrecy::SecretString;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{KeyMembershipDatabase, KeyMembershipStatement, KeySyncError, MovedGroupRow};

const ATTACH_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_append(\"assigned_key_ids\", $1) WHERE \"access_group_id\" = ANY($2::text[]) AND NOT ($1 = ANY(\"assigned_key_ids\")) RETURNING \"access_group_id\"";
const DETACH_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_remove(\"assigned_key_ids\", $1) WHERE \"access_group_id\" = ANY($2::text[]) AND $1 = ANY(\"assigned_key_ids\") RETURNING \"access_group_id\"";
const REPOINT_KEY_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"assigned_key_ids\" = array_append(array_remove(array_remove(\"assigned_key_ids\", $1), $2), $2) WHERE $1 = ANY(\"assigned_key_ids\") RETURNING \"access_group_id\"";
const ACCESS_GROUP_TABLE: &str = <AccessGroupTable as TableBinding>::NAME;

pub struct PostgresKeyMembership<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl KeyMembershipDatabase for PostgresKeyMembership<'_> {
    fn move_key_membership(
        &self,
        statement: KeyMembershipStatement,
        key_token: &str,
        access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send {
        let sql = match statement {
            KeyMembershipStatement::Attach => ATTACH_KEY_SQL,
            KeyMembershipStatement::Detach => DETACH_KEY_SQL,
        };
        let client = self.client;
        async move {
            let parameters: [&(dyn ToSql + Sync); 2] = [&key_token, &access_group_ids];
            moved_groups(client, sql, &parameters).await
        }
    }

    fn repoint_key_membership(
        &self,
        previous_key_token: &str,
        new_key_token: &str,
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send {
        let client = self.client;
        async move {
            let parameters: [&(dyn ToSql + Sync); 2] = [&previous_key_token, &new_key_token];
            moved_groups(client, REPOINT_KEY_SQL, &parameters).await
        }
    }
}

// B714 pins raw SQL writes to the writer handle even while the writer reports degraded
struct WriterStore<'db> {
    store: &'db PostgresStore,
}

impl DatabaseLifecycle for WriterStore<'_> {
    type Error = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn is_connected(&self) -> bool {
        self.store.pool.is_some()
    }

    #[cfg_attr(test, mutants::skip)]
    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(false) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for WriterStore<'_> {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        if name != ACCESS_GROUP_TABLE {
            return Ok(None);
        }
        Ok(self.store.pool.clone().map(Attribute::Model))
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_refresh_enabled(&self) -> bool {
        false
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_label(&self) -> &'static str {
        "access group writer"
    }

    #[cfg_attr(test, mutants::skip)]
    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
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
    let Some(present) = client else {
        return Err(KeySyncError::Database(DbError::NoDbConnected));
    };
    let repository = PrismaTableRepository::new(
        Some(present),
        redis_config_change_publisher(None),
        AccessGroupTable,
    );
    let resolved = repository.prisma_client()?;
    let writer = WriterStore {
        store: resolved.writer,
    };
    let pinned = WriterPinnedClient::Writer(&writer);
    let store = writer_wrapper(pinned).store;
    let session = store.session().await?;
    let rows = session
        .query(sql, parameters)
        .await
        .map_err(|source| KeySyncError::Database(backend(source)))?;
    rows.iter().map(MovedGroupRow::from_row).collect()
}
