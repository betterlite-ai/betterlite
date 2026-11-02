use super::prefix::{PrefixPolicy, strip_known_server_prefix};
use super::types::{MCPServer, MCPTool};

#[must_use]
pub fn apply_description_overrides(
    tools: &[MCPTool],
    server: &MCPServer,
    policy: &PrefixPolicy,
) -> Vec<MCPTool> {
    if server.tool_name_to_description.is_empty() {
        return tools.to_vec();
    }
    tools
        .iter()
        .map(|tool| {
            let stripped = strip_known_server_prefix(&tool.name, Some(server), policy);
            let description = server.tool_name_to_description.get(&stripped);
            described_tool(tool, description)
        })
        .collect()
}

fn described_tool(tool: &MCPTool, override_description: Option<&String>) -> MCPTool {
    match override_description {
        Some(description) if Some(description) != tool.description.as_ref() => MCPTool {
            description: Some(description.clone()),
            ..tool.clone()
        },
        _ => tool.clone(),
    }
}
