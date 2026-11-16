use std::sync::Arc;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use deadpool_postgres::GenericClient;
use uuid::Uuid;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::actions::user_env_var_actions;
use crate::mcp::table_access::rows::{ENV_VAR_TABLE, KeyValues, RowFields};
use crate::mcp::table_access::sql::{self, Select};
use crate::mcp::table_access::store::{LockedTable, McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::McpTableError;
use crate::postgres_store::errors::backend;
use crate::stored_cipher::DbCipher;

use super::error::UserEnvVarsError;
use super::ports::{
    TransactionFuture, UserEnvVarTable, UserEnvVarsMergeStep, UserEnvVarsTransaction,
    UserEnvVarsTransactions,
};
use super::store::{UserEnvVars, UserEnvVarsStore};

// the composite key LiteLLM's find_unique and delete_many both use
pub(crate) fn composite_key(user_id: &str, server_id: &str) -> KeyValues {
    KeyValues::from([
        ("user_id", user_id.to_owned()),
        ("server_id", server_id.to_owned()),
    ])
}

async fn statement_rows<Client>(
    client: &Client,
    statement: &sql::Statement,
) -> Result<Vec<JsonData>, McpTableError>
where
    Client: GenericClient,
{
    sql::rows(client, statement)
        .await
        .map_err(McpTableError::Table)
}

async fn locked_row<Client>(
    client: &Client,
    filter: &KeyValues,
) -> Result<Option<JsonData>, McpTableError>
where
    Client: GenericClient,
{
    let select = Select {
        predicate: sql::key_predicate(filter, 1),
        descending: false,
        take: None,
    };
    let mut rows = statement_rows(client, &sql::select_sql(&ENV_VAR_TABLE, &select)).await?;
    Ok(rows.pop())
}

impl UserEnvVarTable for McpTableStore<'_> {
    async fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<JsonData>, UserEnvVarsError> {
        let filter = composite_key(user_id, server_id);
        let rows = sql::select_keyed(self.reader(), &ENV_VAR_TABLE, Some(&filter)).await?;
        Ok(rows.into_iter().next())
    }

    async fn find_many(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<Vec<JsonData>, UserEnvVarsError> {
        let client = self.reader().session().await.map_err(McpTableError::from)?;
        let query = format!(
            "SELECT row_to_json(t)::text FROM \"{}\" t WHERE t.\"user_id\" = $1 AND t.\"server_id\" = ANY($2::text[])",
            ENV_VAR_TABLE.relation
        );
        let ids = server_ids.to_vec();
        let found = client
            .query(&query, &[&user_id, &ids])
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let mut rows = Vec::with_capacity(found.len());
        for row in &found {
            rows.push(sql::row_data(row).map_err(McpTableError::Table)?);
        }
        Ok(rows)
    }

    async fn delete_many(&self, user_id: &str, server_id: &str) -> Result<(), UserEnvVarsError> {
        let table = user_env_var_actions(Some(self.client), Arc::clone(&self.publish))?;
        let _deleted = table
            .delete_many(composite_key(user_id, server_id))
            .await
            .map_err(McpTableError::Table)?;
        Ok(())
    }
}

impl UserEnvVarsTransaction for LockedTable<'_> {
    fn execute_raw<'tx>(&'tx self, statement: &'tx str, key: i64) -> TransactionFuture<'tx, ()> {
        Box::pin(async move {
            self.transaction
                .execute(statement, &[&key])
                .await
                .map_err(|source| McpTableError::Database(backend(source)))?;
            Ok(())
        })
    }

    fn find_unique<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
    ) -> TransactionFuture<'tx, Option<JsonData>> {
        Box::pin(async move {
            locked_row(self.transaction, &composite_key(user_id, server_id))
                .await
                .map_err(UserEnvVarsError::from)
        })
    }

    fn upsert<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
        values_b64: &'tx str,
    ) -> TransactionFuture<'tx, ()> {
        // the table has no default for id, so the create arm supplies one like Prisma does
        let id = Uuid::new_v4().to_string();
        let data = RowFields::texts(&[
            ("id", id.as_str()),
            ("user_id", user_id),
            ("server_id", server_id),
            ("values_b64", values_b64),
        ]);
        Box::pin(async move {
            let statement =
                sql::insert_sql(&ENV_VAR_TABLE, &data, true).map_err(McpTableError::Table)?;
            drop(statement_rows(self.transaction, &statement).await?);
            Ok(())
        })
    }
}

impl UserEnvVarsTransactions for McpTableStore<'_> {
    fn in_transaction<'tx>(
        &'tx self,
        step: &'tx dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'tx, UserEnvVars> {
        Box::pin(async move {
            let mut manager = db_transaction_manager(self.client.writer).await?;
            let transaction = manager.begin().await?;
            let locked = LockedTable {
                transaction: &transaction,
            };
            let merged = step.apply(&locked).await?;
            transaction
                .commit()
                .await
                .map_err(|source| McpTableError::Database(backend(source)))?;
            Ok(merged)
        })
    }
}

pub type PostgresUserEnvVarsStore<'db> =
    UserEnvVarsStore<McpTableStore<'db>, McpTableStore<'db>, OsNonces, TracingDiagnostics>;

pub fn postgres_user_env_vars_store<'db>(
    client: Option<PrismaClient<'db>>,
    publish: PublishChange<TableError>,
    cipher: DbCipher,
) -> Result<PostgresUserEnvVarsStore<'db>, McpTableError> {
    Ok(UserEnvVarsStore {
        table: McpTableStore::new(client, Arc::clone(&publish))?,
        transactions: McpTableStore::new(client, publish)?,
        cipher,
    })
}
