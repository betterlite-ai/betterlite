use crate::tool_permission::types::{PermissionError, ToolResult};

use super::extraction::ToolCallInfo;

// LiteLLM's _create_permission_error_result: the denial text a rewritten response carries in place of the tool use
#[must_use]
pub fn permission_error_result(call: &ToolCallInfo, error: &PermissionError) -> ToolResult {
    let mut content = format!("Permission denied: {}", error.message);
    if let Some(rule_id) = &error.rule_id {
        content.push_str(&format!(" (Rule: {rule_id})"));
    }
    ToolResult {
        kind: "tool_result".to_owned(),
        tool_use_id: call.id.clone(),
        content,
        is_error: true,
    }
}
