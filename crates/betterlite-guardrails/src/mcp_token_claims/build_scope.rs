use crate::mcp_guardrail_translation::mapping_registration::McpCallType;

mod tests;

const TOOL_CALL_SCOPE: &str = "mcp:tools/call";
const TOOL_LIST_SCOPE: &str = "mcp:tools/list";

#[must_use]
pub fn generate(
    allowed_scopes: Option<&[String]>,
    tool_name: Option<&str>,
    call_type: Option<McpCallType>,
) -> String {
    if let Some(scopes) = allowed_scopes {
        return scopes.join(" ");
    }
    let sanitized: String = tool_name
        .unwrap_or_default()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if !sanitized.is_empty() {
        return format!("{TOOL_CALL_SCOPE} mcp:tools/{sanitized}:call");
    }
    if call_type == Some(McpCallType::CallMcpTool) {
        return TOOL_CALL_SCOPE.to_owned();
    }
    TOOL_LIST_SCOPE.to_owned()
}
