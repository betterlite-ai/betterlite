use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use deadpool_postgres::{Client, GenericClient as _, Transaction};

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::actions::_mcp_server_table_actions;
use super::filter::FindServers;
use super::rows::{KeyValues, SERVER_TABLE, ServerUpdate};
use super::sql::{self, Scope, Select};
use super::types::{McpServerAccess, McpTableError, ServerWrite};

pub struct McpTableStore<'db> {
    pub(crate) client: PrismaClient<'db>,
    pub(crate) publish: PublishChange<TableError>,
}

impl<'db> McpTableStore<'db> {
    pub fn new(
        client: Option<PrismaClient<'db>>,
        publish: PublishChange<TableError>,
    ) -> Result<Self, McpTableError> {
        let Some(connected) = client else {
            return Err(McpTableError::from(DbError::NoDbConnected));
        };
        Ok(Self {
            client: connected,
            publish,
        })
    }

    #[must_use]
    pub fn reader(&self) -> &'db PostgresStore {
        if self
            .client
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
        {
            return self.client.writer;
        }
        self.client.reader
    }
}

impl From<&FindServers> for Select {
    fn from(request: &FindServers) -> Self {
        Self {
            predicate: request.filter.as_ref().map(|filter| filter.predicate(1)),
            descending: request.descending,
            take: request.take,
        }
    }
}

impl McpServerAccess for McpTableStore<'_> {
    async fn find_servers(&self, request: &FindServers) -> Result<Vec<JsonData>, McpTableError> {
        let client = self.reader().session().await?;
        let statement = sql::select_sql(&SERVER_TABLE, &Select::from(request));
        sql::rows(&client, &statement)
            .await
            .map_err(McpTableError::Table)
    }

    async fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> Result<Option<JsonData>, McpTableError> {
        let table = _mcp_server_table_actions(Some(self.client), Arc::clone(&self.publish))?;
        let filter = KeyValues::from([("server_id", request.server_id.clone())]);
        table
            .update(request.data.clone(), filter)
            .await
            .map_err(McpTableError::Table)
    }
}

// the server table seen through an open transaction, as LiteLLM's tx.litellm_mcpservertable
pub(crate) struct LockedTable<'tx> {
    pub(crate) transaction: &'tx Transaction<'tx>,
}

impl LockedTable<'_> {
    pub(crate) async fn write(
        &self,
        write: &ServerWrite,
    ) -> Result<Option<JsonData>, McpTableError> {
        let statement = match write {
            ServerWrite::Create(data) => sql::insert_sql(&SERVER_TABLE, data, false)?,
            ServerWrite::Update(update) => {
                let filter = KeyValues::from([("server_id", update.server_id.clone())]);
                sql::update_statement(&SERVER_TABLE, &update.data, &filter, Scope::Unique)?
            }
        };
        Ok(sql::rows(self.transaction, &statement).await?.pop())
    }
}

impl McpServerAccess for LockedTable<'_> {
    async fn find_servers(&self, request: &FindServers) -> Result<Vec<JsonData>, McpTableError> {
        let statement = sql::select_sql(&SERVER_TABLE, &Select::from(request));
        sql::rows(self.transaction, &statement)
            .await
            .map_err(McpTableError::Table)
    }

    async fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> Result<Option<JsonData>, McpTableError> {
        self.write(&ServerWrite::Update(request.clone())).await
    }
}

pub struct TransactionManager {
    client: Client,
}

impl TransactionManager {
    pub async fn begin(&mut self) -> Result<Transaction<'_>, McpTableError> {
        self.client
            .transaction()
            .await
            .map_err(|source| McpTableError::Database(backend(source)))
    }
}

pub async fn _db_transaction_manager(
    store: &PostgresStore,
) -> Result<TransactionManager, McpTableError> {
    Ok(TransactionManager {
        client: store.session().await?,
    })
}
