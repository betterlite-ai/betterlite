use std::num::FpCategory;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::tool_request::McpToolRequest;

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
    pub fn tool_arguments(&self) -> Option<&GuardrailValue> {
        match &self.mcp_arguments {
            Some(arguments) if is_truthy(arguments) => Some(arguments),
            Some(_) | None => self.arguments.as_ref(),
        }
    }
}

fn is_truthy(value: &GuardrailValue) -> bool {
    match value {
        GuardrailValue::Null => false,
        GuardrailValue::Boolean(flag) => *flag,
        GuardrailValue::SignedInteger(number) => *number != 0,
        GuardrailValue::UnsignedInteger(number) => *number != 0,
        GuardrailValue::Decimal(number) => number.classify() != FpCategory::Zero,
        GuardrailValue::Text(text) => !text.is_empty(),
        GuardrailValue::Array(items) => !items.is_empty(),
        GuardrailValue::Object(entries) => !entries.is_empty(),
    }
}
