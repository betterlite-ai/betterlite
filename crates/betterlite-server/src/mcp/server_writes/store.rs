use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::actions::{
    mcp_server_table_actions, oauth_client_table_actions, user_credential_actions,
    user_env_var_actions,
};
use betterlite_db::mcp::table_access::db_find_mcp_server_rows;
use betterlite_db::mcp::table_access::filter::ServerFilter;
// ast-grep-ignore: no-long-names
use betterlite_db::mcp::table_access::lock::mcp_server_write_if_identifier_free;
use betterlite_db::mcp::table_access::rows::{KeyValues, RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::server_reads::db_find_mcp_server_row;
use betterlite_db::mcp::table_access::store::McpTableStore;
use betterlite_db::mcp::table_access::types::{
    LockedWrite, LockedWriteOutcome, McpServerAccess as _, McpTableError,
};
use betterlite_db::mcp::table_access::user_tables::db_find_user_credential_rows;
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions as _;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

use super::decode::row_as;
use super::{ChildTable, DRAFT, ServerWriteStore};

// get_draft_mcp_server enforces the TTL as part of the read, so an expired draft never comes back.
const RECENT_DRAFT_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_MCPServerTable" t WHERE t."server_id" = $1 AND t."approval_status" = 'draft' AND t."updated_at" >= $2::text::timestamptz LIMIT 1"#;

// The Postgres side of ServerWriteStore, built from the MCP table access layer of betterlite-db.
pub struct PostgresServerWrites<'db> {
    pub client: PrismaClient<'db>,
    pub publish: PublishChange<TableError>,
}

#[derive(Deserialize)]
struct CredentialRow {
    user_id: String,
}

impl ServerWriteStore for PostgresServerWrites<'_> {
    async fn find_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        let store = McpTableStore::new(Some(self.client), Arc::clone(&self.publish))?;
        db_find_mcp_server_row(&store, server_id).await
    }

    async fn find_drafts(&self) -> Result<Vec<JsonData>, McpTableError> {
        let filter = ServerFilter::ApprovalStatus(DRAFT.to_owned());
        let store = McpTableStore::new(Some(self.client), Arc::clone(&self.publish))?;
        db_find_mcp_server_rows(&store, Some(filter)).await
    }

    async fn find_draft_updated_since(
        &self,
        server_id: &str,
        cutoff: DateTime<Utc>,
    ) -> Result<Option<JsonData>, McpTableError> {
        // The draft must be visible to the worker that just wrote it, so this reads the writer.
        let client = self.client.writer.session().await?;
        let since = cutoff.to_rfc3339_opts(SecondsFormat::Micros, true);
        let rows = client
            .query(RECENT_DRAFT_SQL, &[&server_id, &since])
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let parsed = serde_json::from_str(&text).map_err(TableError::Row)?;
        Ok(Some(parsed))
    }

    async fn create_server(&self, data: RowFields) -> Result<JsonData, McpTableError> {
        let table = mcp_server_table_actions(Some(self.client), Arc::clone(&self.publish))?;
        Ok(table.create(data).await?)
    }

    async fn update_server(&self, update: ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        let store = McpTableStore::new(Some(self.client), Arc::clone(&self.publish))?;
        store.update_server(&update).await
    }

    async fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> Result<LockedWriteOutcome, McpTableError> {
        let store = McpTableStore::new(Some(self.client), Arc::clone(&self.publish))?;
        // ast-grep-ignore: no-long-names
        mcp_server_write_if_identifier_free(&store, &request).await
    }

    async fn delete_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        let table = mcp_server_table_actions(Some(self.client), Arc::clone(&self.publish))?;
        let filter = KeyValues::from([("server_id", server_id.to_owned())]);
        Ok(table.delete(filter).await?)
    }

    async fn credential_user_ids(&self, server_id: &str) -> Result<Vec<String>, McpTableError> {
        let filter = KeyValues::from([("server_id", server_id.to_owned())]);
        let store = McpTableStore::new(Some(self.client), Arc::clone(&self.publish))?;
        let rows = db_find_user_credential_rows(&store, Some(&filter)).await?;
        let mut user_ids = Vec::with_capacity(rows.len());
        for row in &rows {
            user_ids.push(row_as::<CredentialRow>(row)?.user_id);
        }
        Ok(user_ids)
    }

    async fn delete_children(
        &self,
        table: ChildTable,
        server_id: &str,
    ) -> Result<(), McpTableError> {
        let filter = KeyValues::from([("server_id", server_id.to_owned())]);
        match table {
            ChildTable::Credential => {
                let actions =
                    user_credential_actions(Some(self.client), Arc::clone(&self.publish))?;
                actions.delete_many(filter).await?;
            }
            ChildTable::EnvVar => {
                let actions = user_env_var_actions(Some(self.client), Arc::clone(&self.publish))?;
                actions.delete_many(filter).await?;
            }
            ChildTable::OAuthClient => {
                let actions =
                    oauth_client_table_actions(Some(self.client), Arc::clone(&self.publish))?;
                actions.delete_many(filter).await?;
            }
        }
        Ok(())
    }
}
