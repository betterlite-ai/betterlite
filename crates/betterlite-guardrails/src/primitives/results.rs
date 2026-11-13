use indexmap::IndexMap;
use serde::Serialize;

use super::json_value::JsonValue;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActionResult {
    Allow,
    Block {
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detection_info: Option<IndexMap<String, JsonValue>>,
    },
    Flag {
        reason: String,
        metadata: IndexMap<String, JsonValue>,
    },
    Modify {
        #[serde(skip_serializing_if = "Option::is_none")]
        texts: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        images: Option<Vec<JsonValue>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<JsonValue>>,
    },
}
