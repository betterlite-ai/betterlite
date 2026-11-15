use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions};
use deadpool_postgres::GenericClient as _;
use tokio_postgres::Row;

use crate::auxiliary_repository::bindings;
use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

use super::rows::{
    CREDENTIAL_TABLE, ENV_VAR_TABLE, KeyValues, McpUserEnvVarsRepository, OAUTH_CLIENT_TABLE,
    RowFields, SERVER_TABLE, Table,
};
use super::sql;
use super::types::McpTableError;

pub type Handle<'db> = Result<ConfigSyncTableActions<McpTableActions<'db>>, McpTableError>;

pub struct McpTableActions<'db> {
    pub(crate) store: &'db PostgresStore,
    pub(crate) table: &'static Table,
}

impl McpTableActions<'_> {
    async fn rows(&self, query: &str, binds: &[String]) -> Result<Vec<Row>, TableError> {
        let client = self.store.session().await?;
        client
            .query(query, &sql::params(binds))
            .await
            .map_err(|source| TableError::Database(backend(source)))
    }

    async fn write(&self, query: &str, binds: &[String]) -> Result<u64, TableError> {
        let client = self.store.session().await?;
        client
            .execute(query, &sql::params(binds))
            .await
            .map_err(|source| TableError::Database(backend(source)))
    }

    async fn one(&self, query: &str, binds: &[String]) -> Result<JsonData, TableError> {
        let rows = self.rows(query, binds).await?;
        let Some(row) = rows.first() else {
            return Err(TableError::MissingRow);
        };
        sql::row_data(row)
    }
}

impl TableActions for McpTableActions<'_> {
    type Row = JsonData;
    type Filter = KeyValues;
    type CreateData = RowFields;
    type UpdateData = RowFields;
    type Failure = TableError;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        let (query, binds) = sql::insert_sql(self.table, &data, false)?;
        self.one(&query, &binds).await
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        let count = data.len();
        let mut client = self.store.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        for entry in &data {
            let (query, binds) = sql::insert_sql(self.table, entry, false)?;
            transaction
                .execute(&query, &sql::params(&binds))
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
        }
        transaction
            .commit()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(count)
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        if filter.is_empty() {
            return Ok(None);
        }
        let (query, binds) = sql::update_statement(self.table, &data, &filter, true)?;
        let rows = self.rows(&query, &binds).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        sql::row_data(row).map(Some)
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        if filter.is_empty() {
            return Ok(0);
        }
        let (query, binds) = sql::update_statement(self.table, &data, &filter, false)?;
        let rows = self.write(&query, &binds).await?;
        usize::try_from(rows).map_err(|source| TableError::RowCount { source })
    }

    async fn upsert(
        &self,
        _filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        let (query, binds) = sql::insert_sql(self.table, &data, true)?;
        self.one(&query, &binds).await
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        if filter.is_empty() {
            return Ok(None);
        }
        let (query, binds) = sql::delete_statement(self.table, &filter, true);
        let rows = self.rows(&query, &binds).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        sql::row_data(row).map(Some)
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        if filter.is_empty() {
            return Ok(0);
        }
        let (query, binds) = sql::delete_statement(self.table, &filter, false);
        let rows = self.write(&query, &binds).await?;
        usize::try_from(rows).map_err(|source| TableError::RowCount { source })
    }
}

fn actions<'db, Binding: TableBinding>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
    binding: Binding,
    table: &'static Table,
) -> Handle<'db> {
    PrismaTableRepository::new(client, publish, binding)
        .table(|access| McpTableActions {
            store: access.write,
            table,
        })
        .map_err(McpTableError::from)
}

// ast-grep-ignore: no-one-line-functions
pub fn _mcp_server_table_actions<'db>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
) -> Handle<'db> {
    actions(client, publish, bindings::MCPServerTable, &SERVER_TABLE)
}

pub fn _oauth_client_table_actions<'db>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
) -> Handle<'db> {
    actions(
        client,
        publish,
        bindings::MCPServerOAuthClient,
        &OAUTH_CLIENT_TABLE,
    )
}

pub fn _user_credential_actions<'db>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
) -> Handle<'db> {
    actions(
        client,
        publish,
        bindings::MCPUserCredentials,
        &CREDENTIAL_TABLE,
    )
}

// ast-grep-ignore: no-one-line-functions
pub fn _user_env_var_actions<'db>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
) -> Handle<'db> {
    actions(client, publish, McpUserEnvVarsRepository, &ENV_VAR_TABLE)
}
