use betterlite_server::mcp::catalog_guard::types::MCPTool;

use super::{FromFixture, ToolInput};

impl FromFixture<ToolInput> for MCPTool {
    fn from_fixture(input: &ToolInput) -> Self {
        Self {
            name: input.name.clone(),
            description: input.description.clone(),
            title: None,
            input_schema: input.input_schema.clone(),
            output_schema: input.output_schema.clone(),
            execution: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }
}
