use std::sync::Arc;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use deadpool_postgres::GenericClient as _;
use uuid::Uuid;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::actions::user_env_var_actions;
use crate::mcp::table_access::rows::{ENV_VAR_TABLE, KeyValues, RowFields};
use crate::mcp::table_access::sql::{self, Select};
use crate::mcp::table_access::store::{LockedTable, McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::McpTableError;
use crate::mcp::table_access::user_tables::db_find_user_env_var_rows;
use crate::postgres_store::errors::backend;
use crate::stored_cipher::DbCipher;

use super::error::UserEnvVarsError;
use super::ports::{
    TransactionFuture, UserEnvVarTable, UserEnvVarsMergeStep, UserEnvVarsTransaction,
    UserEnvVarsTransactions,
};
use super::store::{UserEnvVars, UserEnvVarsStore};

// the composite key LiteLLM's find_unique and delete_many both use
fn composite_key(user_id: &str, server_id: &str) -> KeyValues {
    KeyValues::from([
        ("user_id", user_id.to_owned()),
        ("server_id", server_id.to_owned()),
    ])
}

impl UserEnvVarTable for McpTableStore<'_> {
    async fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<JsonData>, UserEnvVarsError> {
        let key = composite_key(user_id, server_id);
        Ok(db_find_user_env_var_rows(self, Some(&key)).await?.pop())
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

    async fn delete_many(&self, user_id: &str, server_id: &str) -> Result<usize, UserEnvVarsError> {
        let table = user_env_var_actions(Some(self.client), Arc::clone(&self.publish))?;
        Ok(table
            .delete_many(composite_key(user_id, server_id))
            .await
            .map_err(McpTableError::Table)?)
    }
}

impl UserEnvVarsTransaction for LockedTable<'_> {
    fn execute_raw<'a>(&'a self, statement: &'a str, key: i64) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            self.transaction
                .execute(statement, &[&key])
                .await
                .map_err(|source| McpTableError::Database(backend(source)))?;
            Ok(())
        })
    }

    fn find_unique<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
    ) -> TransactionFuture<'a, Option<JsonData>> {
        Box::pin(async move {
            let select = Select {
                predicate: sql::key_predicate(&composite_key(user_id, server_id), 1),
                descending: false,
                take: None,
            };
            let statement = sql::select_sql(&ENV_VAR_TABLE, &select);
            Ok(sql::rows(self.transaction, &statement).await?.pop())
        })
    }

    fn upsert<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
        values_b64: &'a str,
    ) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            // the table has no default for id, so the create arm supplies one like Prisma does
            let id = Uuid::new_v4().to_string();
            let data = RowFields::texts(&[
                ("id", id.as_str()),
                ("user_id", user_id),
                ("server_id", server_id),
                ("values_b64", values_b64),
            ]);
            let statement =
                sql::insert_sql(&ENV_VAR_TABLE, &data, true).map_err(McpTableError::Table)?;
            drop(
                sql::rows(self.transaction, &statement)
                    .await
                    .map_err(McpTableError::Table)?,
            );
            Ok(())
        })
    }
}

impl UserEnvVarsTransactions for McpTableStore<'_> {
    fn in_transaction<'a>(
        &'a self,
        step: &'a dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'a, UserEnvVars> {
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
