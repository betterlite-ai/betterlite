use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use deadpool_postgres::{Client, GenericClient as _};
use tokio_postgres::GenericClient as _;
use tokio_postgres::Transaction as PgTransaction;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::actions::_mcp_server_table_actions;
use super::filter::{FindServers, identifier_candidates};
use super::rows::{KeyValues, RowFields, SERVER_TABLE, ServerUpdate, server_id_of};
use super::sql;
use super::types::{LockedWrite, LockedWriteOutcome, McpServerAccess, McpTableError, ServerWrite};
use super::{
    _identifier_where, _mcp_identifier_lock_keys, ADVISORY_LOCK_SQL, McpIdentifierConflict,
};

pub struct McpTableStore<'db> {
    pub(crate) client: PrismaClient<'db>,
    pub(crate) publish: PublishChange<TableError>,
}

impl<'db> McpTableStore<'db> {
    pub fn new(
        client: Option<PrismaClient<'db>>,
        publish: PublishChange<TableError>,
    ) -> Result<Self, McpTableError> {
        let Some(client) = client else {
            return Err(McpTableError::from(DbError::NoDbConnected));
        };
        Ok(Self { client, publish })
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

    pub async fn write_if_identifier_free(
        &self,
        request: &LockedWrite,
    ) -> Result<LockedWriteOutcome, McpTableError> {
        let mut client = self.client.writer.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let identifiers = [request.server_name.as_deref(), request.alias.as_deref()];
        for key in _mcp_identifier_lock_keys(&identifiers) {
            let params: [&(dyn ToSql + Sync); 1] = [&key];
            transaction
                .execute(ADVISORY_LOCK_SQL, &params)
                .await
                .map_err(|source| McpTableError::Database(backend(source)))?;
        }
        if let Some(conflict) = locked_conflict(&transaction, request).await? {
            return Ok(LockedWriteOutcome::Conflict(conflict));
        }
        let written = apply_write(&transaction, &request.write).await?;
        transaction
            .commit()
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        (self.publish)(SERVER_TABLE.name)
            .await
            .map_err(McpTableError::Table)?;
        Ok(LockedWriteOutcome::Written(written))
    }
}

impl McpServerAccess for McpTableStore<'_> {
    async fn find_servers(&self, request: &FindServers) -> Result<Vec<JsonData>, McpTableError> {
        sql::select_servers(self.reader(), &SERVER_TABLE, request).await
    }

    async fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> Result<Option<JsonData>, McpTableError> {
        let table = _mcp_server_table_actions(Some(self.client), Arc::clone(&self.publish))?;
        let filter = KeyValues::from([("server_id", request.server_id.clone())]);
        let row = table.update(request.data.clone(), filter).await?;
        Ok(row)
    }
}

async fn locked_conflict(
    transaction: &PgTransaction<'_>,
    request: &LockedWrite,
) -> Result<Option<McpIdentifierConflict>, McpTableError> {
    let candidates =
        identifier_candidates(request.server_name.as_deref(), request.alias.as_deref());
    for (field, value) in candidates {
        let filter = _identifier_where(value, request.exclude_server_id.as_deref());
        let Some((predicate, binds)) = filter.predicate(1) else {
            continue;
        };
        let select = sql::Select {
            predicate: Some(predicate),
            binds: &binds,
            descending: false,
            take: Some(1),
        };
        let query = sql::select_sql(&SERVER_TABLE, &select);
        let row = transaction
            .query_opt(&query, &sql::params(&binds))
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let Some(row) = row else {
            continue;
        };
        return Ok(Some(McpIdentifierConflict {
            field,
            value: value.to_owned(),
            server_id: server_id_of(&sql::row_data(&row)?)?,
        }));
    }
    Ok(None)
}

async fn apply_write(
    transaction: &PgTransaction<'_>,
    write: &ServerWrite,
) -> Result<Option<JsonData>, McpTableError> {
    let (query, binds) = match write {
        ServerWrite::Create(data) => sql::insert_sql(&SERVER_TABLE, data, false)?,
        ServerWrite::Update { server_id, data } => {
            let filter = KeyValues::from([("server_id", server_id.clone())]);
            sql::update_statement(&SERVER_TABLE, data, &filter, true)?
        }
    };
    let row = transaction
        .query_opt(&query, &sql::params(&binds))
        .await
        .map_err(|source| McpTableError::Database(backend(source)))?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(sql::row_data(&row)?))
}

pub struct TransactionManager {
    client: Client,
}

impl TransactionManager {
    pub async fn begin(&mut self) -> Result<deadpool_postgres::Transaction<'_>, McpTableError> {
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
