pub mod create;
pub mod decode;
pub mod delete;
pub mod draft;
pub mod prepare;
pub mod store;
pub mod update;

mod tests;

use std::future::Future;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{LockedWrite, LockedWriteOutcome, McpTableError};
use betterlite_db::mcp_server_encoding::McpEncodingError;
use chrono::{DateTime, Utc};

// The approval_status that marks the transient rows backing the admin OAuth session flow.
pub const DRAFT: &str = "draft";

#[derive(Debug, thiserror::Error)]
pub enum ServerWriteError {
    #[error(transparent)]
    Table(#[from] McpTableError),
    #[error(transparent)]
    Encoding(#[from] McpEncodingError),
    #[error("the MCP server value cannot be encoded as JSON")]
    Json(#[source] serde_json::Error),
    #[error("MCP server {server_id} was written but its stored row cannot be read back")]
    UnreadableRow { server_id: String },
    #[error("the prepared MCP server payload names a column betterlite does not map: {name}")]
    UnknownColumn { name: String },
    #[error("the inserted MCP server row is missing")]
    InsertedRowMissing,
    #[error("the MCP server update has no server_id")]
    MissingServerId,
}

// LiteLLM's create_draft_mcp_server takes the acting user, the TTL and an optional caller-chosen id.
#[derive(Debug, Clone, Default)]
pub struct DraftOptions {
    pub touched_by: String,
    pub ttl_seconds: i64,
    pub server_id: Option<String>,
}

// LiteLLM's create_mcp_server_if_identifier_free returns the written row or the collision it found.
#[derive(Debug)]
pub enum CreatedServer {
    Created(Box<McpServerTable>),
    Conflict(Box<McpIdentifierConflict>),
}

// LiteLLM's update_mcp_server returns the updated row, the collision, or None for a missing row.
#[derive(Debug)]
pub enum UpdatedServer {
    Updated(Option<Box<McpServerTable>>),
    Conflict(Box<McpIdentifierConflict>),
}

// The identity a caller needs after a delete: the registry evicts by server_id and server_name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedServer {
    pub server_id: String,
    pub server_name: Option<String>,
}

// The three per-server tables that carry no foreign key to the server row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildTable {
    Credential,
    EnvVar,
    OAuthClient,
}

impl ChildTable {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Credential => "credential",
            Self::EnvVar => "env var",
            Self::OAuthClient => "OAuth client",
        }
    }
}

// The per-user token cache invalidation delete_mcp_server calls for every enumerated credential user.
pub trait TokenCacheInvalidation: Send + Sync {
    fn invalidate(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), TokenCacheError>> + Send;
}

// A downstream cache reports why it could not drop the entry; delete only logs it.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct TokenCacheError {
    pub message: String,
}

// The row store the write paths run against, so the orchestration is testable without a database.
pub trait ServerWriteStore: Send + Sync {
    fn find_server(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;

    fn find_drafts(&self) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send;

    fn find_draft_updated_since(
        &self,
        server_id: &str,
        cutoff: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;

    fn create_server(
        &self,
        data: RowFields,
    ) -> impl Future<Output = Result<JsonData, McpTableError>> + Send;

    fn update_server(
        &self,
        update: ServerUpdate,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;

    fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> impl Future<Output = Result<LockedWriteOutcome, McpTableError>> + Send;

    fn delete_server(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;

    fn credential_user_ids(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Vec<String>, McpTableError>> + Send;

    fn delete_children(
        &self,
        table: ChildTable,
        server_id: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send;
}
