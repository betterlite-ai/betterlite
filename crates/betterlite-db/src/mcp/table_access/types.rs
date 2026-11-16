use std::future::Future;

use thiserror::Error;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::ManagedRowsDecodeError;
use crate::managed_resource_rows::mcp::McpServerTable;
use crate::postgres_store::errors::DbError;

use super::filter::FindServers;
use super::rows::{RowFields, ServerUpdate};

pub const PENDING_REVIEW: &str = "pending_review";
pub const ACTIVE: &str = "active";
pub const REJECTED: &str = "rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierField {
    Alias,
    ServerName,
}

impl IdentifierField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alias => "alias",
            Self::ServerName => "server_name",
        }
    }
}

#[derive(Debug, Error)]
pub enum McpTableError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Decode(#[from] ManagedRowsDecodeError),
    #[error("the MCP table row could not be encoded as JSON")]
    Json(#[source] serde_json::Error),
    #[error("the stored MCP server row has no server_id")]
    MissingServerId,
    #[error("MCP server not found, passed server_id={server_id}")]
    ServerNotFound { server_id: String },
}

#[derive(Debug, Clone)]
pub struct MCPSubmissionsSummary {
    pub total: usize,
    pub pending_review: usize,
    pub active: usize,
    pub rejected: usize,
    pub items: Vec<McpServerTable>,
}

pub enum ServerWrite {
    Create(RowFields),
    Update(ServerUpdate),
}

pub struct LockedWrite {
    pub server_name: Option<String>,
    pub alias: Option<String>,
    pub exclude_server_id: Option<String>,
    pub write: ServerWrite,
}

pub enum LockedWriteOutcome {
    Written(Option<JsonData>),
    Conflict(super::McpIdentifierConflict),
}

pub trait McpServerAccess: Send + Sync {
    fn find_servers(
        &self,
        request: &FindServers,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send;

    fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;
}
