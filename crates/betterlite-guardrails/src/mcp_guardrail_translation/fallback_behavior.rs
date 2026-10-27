use super::json_value::McpValue;
use super::tool_request::McpToolRequest;
use std::num::FpCategory;

mod tests;
// LiteLLM reads `mcp_tool_name or name`, so an empty value counts as absent
impl McpToolRequest {
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        match self.mcp_tool_name.as_deref() {
            Some(name) if !name.is_empty() => Some(name),
            Some(_) | None => self.name.as_deref().filter(|name| !name.is_empty()),
        }
    }

    #[must_use]
    pub fn tool_description(&self) -> Option<&str> {
        match self.mcp_tool_description.as_deref() {
            Some(description) if !description.is_empty() => Some(description),
            Some(_) | None => self
                .description
                .as_deref()
                .filter(|description| !description.is_empty()),
        }
    }

    #[must_use]
    pub fn tool_arguments(&self) -> Option<&McpValue> {
        match &self.mcp_arguments {
            Some(arguments) if is_truthy(arguments) => Some(arguments),
            Some(_) | None => self.arguments.as_ref(),
        }
    }
}

fn is_truthy(value: &McpValue) -> bool {
    match value {
        McpValue::Null => false,
        McpValue::Boolean(flag) => *flag,
        McpValue::Number(number) => number.as_f64().map(f64::classify) != Some(FpCategory::Zero),
        McpValue::Text(text) => !text.is_empty(),
        McpValue::Array(items) => !items.is_empty(),
        McpValue::Object(entries) => !entries.is_empty(),
    }
}
