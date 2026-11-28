use betterlite_db::managed_resource_rows::mcp::McpStatus;
use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Health {
    pub status: McpStatus,
    pub error: Option<String>,
    pub checked_at: DateTime<Utc>,
}
