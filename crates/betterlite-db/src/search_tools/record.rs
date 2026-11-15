use chrono::{DateTime, Utc};

use crate::mcp_stored_contracts::JsonValue;

// JsonValue prints [REDACTED], so the stored litellm_params never reach a log through Debug
#[derive(Debug, Clone, PartialEq)]
pub struct SearchToolRecord {
    pub search_tool_id: String,
    pub search_tool_name: String,
    pub litellm_params: JsonValue,
    pub search_tool_info: Option<JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

mod tests;
