use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_stored_contracts::secret::StoredSecret;

use super::super::error::StoreError;
use super::SearchToolRow;

// LiteLLM's Prisma client writes these columns as JSON strings; a row written here holds the document
fn decode_stored<Stored>(field: &'static str, stored: &JsonValue) -> Result<Stored, StoreError>
where
    Stored: DeserializeOwned,
{
    let text = match stored {
        JsonValue::String(text) => text.clone(),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => {
            serde_json::to_string(stored).map_err(|source| StoreError::Field { field, source })?
        }
    };
    serde_json::from_str(&text).map_err(|source| StoreError::Field { field, source })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchToolParams {
    pub search_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<StoredSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_id: Option<String>,
    pub search_tool_name: String,
    pub litellm_params: SearchToolParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_info: Option<BTreeMap<String, JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl TryFrom<SearchToolRow> for SearchTool {
    type Error = StoreError;

    fn try_from(row: SearchToolRow) -> Result<Self, Self::Error> {
        let search_tool_info = match &row.search_tool_info {
            None | Some(JsonValue::Null) => None,
            Some(stored) => Some(decode_stored("search_tool_info", stored)?),
        };
        Ok(Self {
            search_tool_id: Some(row.search_tool_id),
            search_tool_name: row.search_tool_name,
            litellm_params: decode_stored("litellm_params", &row.litellm_params)?,
            search_tool_info,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        })
    }
}
