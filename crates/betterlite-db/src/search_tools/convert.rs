use chrono::{DateTime, Utc};

use crate::mcp_stored_contracts::JsonValue;

pub mod tool;

mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchToolRow {
    pub search_tool_id: String,
    pub search_tool_name: String,
    pub litellm_params: JsonValue,
    pub search_tool_info: Option<JsonValue>,
    pub created_at: String,
    pub updated_at: String,
}

// Python's datetime.isoformat() drops the fraction when it is zero and otherwise writes six digits
#[must_use]
pub fn isoformat(value: DateTime<Utc>) -> String {
    if value.timestamp_subsec_micros() == 0 {
        value.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        value.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
    }
}
