use betterlite_policy::records::attachment::PolicyAttachmentDBResponse;
use betterlite_policy::registry::PolicySource;
use betterlite_policy::row_adapter::PolicyTable;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRow {
    pub attachment_id: String,
    pub policy_name: String,
    pub scope: Option<String>,
    pub teams: Vec<String>,
    pub keys: Vec<String>,
    pub models: Vec<String>,
    pub tags: Vec<String>,
    pub priority: Option<i32>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
}

pub trait AttachmentTableSource: Send + Sync {
    type Table: PolicyTable<AttachmentRow>;

    fn attachment_table(&self) -> Self::Table;
}

impl From<AttachmentRow> for PolicyAttachmentDBResponse {
    fn from(row: AttachmentRow) -> Self {
        Self {
            attachment_id: row.attachment_id,
            policy_name: row.policy_name,
            scope: row.scope,
            teams: row.teams,
            keys: row.keys,
            models: row.models,
            tags: row.tags,
            priority: row.priority,
            default: row.is_default,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
            created_by: row.created_by,
            updated_by: row.updated_by,
            definition_location: PolicySource::Db,
        }
    }
}
