#![cfg(test)]

use crate::tools::error::ToolFormatError;
use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

mod corpus;
mod depth;
mod errors;
mod json;
mod schema;
mod shapes;

fn format(tools_json: &str) -> Result<String, ToolFormatError> {
    let tools = parse_tool_definitions(tools_json.as_bytes(), ToolLimits::default())?;
    format_function_definitions(&tools)
}

fn wrapper(inner: &[&str]) -> String {
    let mut lines = vec!["namespace functions {", ""];
    lines.extend(inner.iter().copied());
    lines.push("} // namespace functions");
    lines.join("\n")
}
