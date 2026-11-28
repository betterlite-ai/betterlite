use std::future::Future;
use std::sync::{Arc, RwLock};

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use thiserror::Error;

use crate::config_loader::ports::OpenApiToolSink;
use crate::registry::tool_owner::ToolOwners;
use crate::server_record::MCPServer;
use crate::tools::schemas::McpTool;
use crate::wire_compat::errors::UpstreamAuthError;

pub type SharedToolOwners = Arc<RwLock<ToolOwners>>;

pub trait ServerRows: Send + Sync {
    fn find_active(
        &self,
    ) -> impl Future<Output = Result<Vec<McpServerTable>, McpTableError>> + Send;
}

pub trait ToolRegistrySink: OpenApiToolSink {
    fn unregister_prefix(&self, prefix: &str) -> usize;
}

pub trait CacheInvalidator: Send + Sync {
    fn invalidate_server(&self, server_id: &str);
}

#[derive(Debug, Error)]
pub enum ToolListError {
    #[error(transparent)]
    UpstreamAuth(#[from] UpstreamAuthError),
    #[error("{0}")]
    Failed(String),
}

pub trait ToolLister: Send + Sync {
    fn list_tools(
        &self,
        server: &MCPServer,
    ) -> impl Future<Output = Result<Vec<McpTool>, ToolListError>> + Send;
}
