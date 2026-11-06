use serde_json::value::RawValue;

mod definition;
pub mod error;
mod schema;
mod tests;

const DEFAULT_MAX_TOOL_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_TOOL_ENTRIES: usize = 1024;

#[derive(Debug, Clone, Copy)]
pub struct ToolLimits {
    pub max_bytes: usize,
    pub max_tools: usize,
}

impl Default for ToolLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_TOOL_BYTES,
            max_tools: DEFAULT_MAX_TOOL_ENTRIES,
        }
    }
}

pub fn parse_tool_definitions(
    bytes: &[u8],
    limits: ToolLimits,
) -> Result<Vec<Box<RawValue>>, error::ToolFormatError> {
    if bytes.len() > limits.max_bytes {
        return Err(error::ToolFormatError::TooLarge {
            bytes: bytes.len(),
            limit: limits.max_bytes,
        });
    }
    let tools: Vec<Box<RawValue>> =
        serde_json::from_slice(bytes).map_err(|source| error::ToolFormatError::Json { source })?;
    if tools.len() > limits.max_tools {
        return Err(error::ToolFormatError::TooMany {
            count: tools.len(),
            limit: limits.max_tools,
        });
    }
    Ok(tools)
}

pub fn format_function_definitions(
    tools: &[Box<RawValue>],
) -> Result<String, error::ToolFormatError> {
    let mut lines = vec!["namespace functions {".to_owned(), String::new()];
    for tool in tools {
        definition::format_tool(tool, &mut lines)?;
    }
    lines.push("} // namespace functions".to_owned());
    Ok(lines.join("\n"))
}
