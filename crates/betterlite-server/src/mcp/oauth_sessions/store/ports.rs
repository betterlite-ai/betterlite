use std::future::Future;
use std::pin::Pin;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_server_encoding::request::McpServerRequest;
use betterlite_mcp::server_build::BuildError;
use betterlite_mcp::server_record::MCPServer;
use uuid::Uuid;

use crate::mcp::server_writes::{DraftOptions, ServerWriteError};

// The table read, the draft read and the draft write the store runs, so a test supplies an in-memory one
pub trait SessionDatabase: Send + Sync {
    fn get_server<'db>(
        &'db self,
        server_id: &'db str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<McpServerTable>, McpTableError>> + Send + 'db>>;

    fn get_draft<'db>(
        &'db self,
        server_id: &'db str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<McpServerTable>, ServerWriteError>> + Send + 'db>>;

    fn create_draft<'db>(
        &'db self,
        request: &'db McpServerRequest,
        options: &'db DraftOptions,
    ) -> Pin<Box<dyn Future<Output = Result<McpServerTable, ServerWriteError>> + Send + 'db>>;
}

// B6763's builder needs collaborators the store does not hold, so B8622 hands it over as one port
pub trait ServerFromTable: Send + Sync {
    fn build_server<'build>(
        &'build self,
        table: &'build McpServerTable,
        credentials_are_encrypted: bool,
    ) -> Pin<Box<dyn Future<Output = Result<MCPServer, BuildError>> + Send + 'build>>;
}

// The fresh id a session gets when the supplied one cannot be kept; tests fix it
pub trait SessionIds: Send + Sync {
    fn fresh_id(&self) -> String;
}

pub struct UuidSessionIds;

impl SessionIds for UuidSessionIds {
    fn fresh_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
