use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions};
use deadpool_postgres::GenericClient as _;

use crate::auxiliary_repository::bindings::{
    MCPServerOAuthClient, MCPServerTable, MCPUserCredentials,
};
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
use super::sql::{self, Scope};
use super::types::McpTableError;

pub type Handle<'db> = Result<ConfigSyncTableActions<McpTableActions<'db>>, McpTableError>;

pub struct McpTableActions<'db> {
    pub(crate) store: &'db PostgresStore,
    pub(crate) table: &'static Table,
}

impl TableActions for McpTableActions<'_> {
    type Row = JsonData;
    type Filter = KeyValues;
    type CreateData = RowFields;
    type UpdateData = RowFields;
    type Failure = TableError;

    async fn create(&self, data: RowFields) -> Result<JsonData, TableError> {
        let mut created = sql::run(self.store, sql::insert_sql(self.table, &data, false)).await?;
        created.pop().ok_or(TableError::MissingRow)
    }

    async fn create_many(&self, data: Vec<RowFields>) -> Result<usize, TableError> {
        let mut client = self.store.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        for entry in &data {
            drop(sql::rows(&transaction, &sql::insert_sql(self.table, entry, false)?).await?);
        }
        transaction
            .commit()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(data.len())
    }

    async fn update(
        &self,
        data: RowFields,
        filter: KeyValues,
    ) -> Result<Option<JsonData>, TableError> {
        let statement = sql::update_statement(self.table, &data, &filter, Scope::Unique);
        Ok(sql::run(self.store, statement).await?.pop())
    }

    async fn update_many(&self, data: RowFields, filter: KeyValues) -> Result<usize, TableError> {
        let statement = sql::update_statement(self.table, &data, &filter, Scope::Many);
        Ok(sql::run(self.store, statement).await?.len())
    }

    async fn upsert(&self, _filter: KeyValues, data: RowFields) -> Result<JsonData, TableError> {
        let mut upserted = sql::run(self.store, sql::insert_sql(self.table, &data, true)).await?;
        upserted.pop().ok_or(TableError::MissingRow)
    }

    async fn delete(&self, filter: KeyValues) -> Result<Option<JsonData>, TableError> {
        let statement = Ok(sql::delete_statement(self.table, &filter, Scope::Unique));
        Ok(sql::run(self.store, statement).await?.pop())
    }

    async fn delete_many(&self, filter: KeyValues) -> Result<usize, TableError> {
        let statement = Ok(sql::delete_statement(self.table, &filter, Scope::Many));
        Ok(sql::run(self.store, statement).await?.len())
    }
}

fn actions<'db, Binding>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
    binding: Binding,
    table: &'static Table,
) -> Handle<'db>
where
    Binding: TableBinding,
{
    PrismaTableRepository::new(client, publish, binding)
        .table(|access| McpTableActions {
            store: access.write,
            table,
        })
        .map_err(McpTableError::from)
}

// ast-grep-ignore: no-one-line-functions
pub fn mcp_server_table_actions(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> Handle<'_> {
    actions(client, publish, MCPServerTable, &SERVER_TABLE)
}

// ast-grep-ignore: no-one-line-functions
pub fn oauth_client_table_actions(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> Handle<'_> {
    actions(client, publish, MCPServerOAuthClient, &OAUTH_CLIENT_TABLE)
}

// ast-grep-ignore: no-one-line-functions
pub fn user_credential_actions(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> Handle<'_> {
    actions(client, publish, MCPUserCredentials, &CREDENTIAL_TABLE)
}

// ast-grep-ignore: no-one-line-functions
pub fn user_env_var_actions(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> Handle<'_> {
    actions(client, publish, McpUserEnvVarsRepository, &ENV_VAR_TABLE)
}
