use betterlite_server::mcp::catalog_guard::types::MCPTool;

use super::{FromFixture, ToolInput};

impl FromFixture<ToolInput> for MCPTool {
    fn from_fixture(tool: &ToolInput) -> Self {
        Self {
            name: tool.name.clone(),
            description: tool.description.clone(),
            title: None,
            input_schema: tool.input_schema.clone(),
            output_schema: tool.output_schema.clone(),
            execution: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }
}
