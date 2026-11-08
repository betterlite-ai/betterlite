use serde::de::IgnoredAny;

use crate::tools::json::Scanner;

mod definition;
pub mod error;
mod json;
mod schema;
mod tests;

const DEFAULT_MAX_TOOL_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_TOOL_ENTRIES: usize = 1024;
// serde_json rejects nesting past 127, which a legal 64-level object schema would reach
const MAX_JSON_DEPTH: usize = 127;

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

#[derive(Debug)]
pub struct ToolDefinitions {
    source: Vec<u8>,
    tools: Vec<Option<definition::Tool>>,
    max_functions: usize,
}

pub fn parse_tool_definitions(
    bytes: &[u8],
    limits: ToolLimits,
) -> Result<ToolDefinitions, error::ToolFormatError> {
    if bytes.len() > limits.max_bytes {
        return Err(error::ToolFormatError::TooLarge {
            bytes: bytes.len(),
            limit: limits.max_bytes,
        });
    }
    if json::max_depth(bytes) > MAX_JSON_DEPTH {
        return Err(error::ToolFormatError::TooDeep {
            limit: schema::MAX_SCHEMA_DEPTH,
        });
    }
    // serde owns JSON syntax: the scanner below only walks what serde accepted
    let _validated: Vec<IgnoredAny> =
        serde_json::from_slice(bytes).map_err(|source| error::ToolFormatError::Json { source })?;
    let tools = scan_tools(bytes)?;
    if tools.len() > limits.max_tools {
        return Err(error::ToolFormatError::TooMany {
            count: tools.len(),
            limit: limits.max_tools,
        });
    }
    Ok(ToolDefinitions {
        source: bytes.to_vec(),
        tools,
        max_functions: limits.max_tools,
    })
}

fn scan_tools(bytes: &[u8]) -> Result<Vec<Option<definition::Tool>>, error::ToolFormatError> {
    let mut scanner = Scanner { bytes, at: 0 };
    let mut tools = Vec::new();
    scanner.take_byte(b'[')?;
    if scanner.peek() == Some(b']') {
        scanner.take_byte(b']')?;
        return Ok(tools);
    }
    loop {
        tools.push(definition::parse_tool(&mut scanner)?);
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b']') => {
                scanner.take_byte(b']')?;
                return Ok(tools);
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
}

pub fn format_function_definitions(
    definitions: &ToolDefinitions,
) -> Result<String, error::ToolFormatError> {
    let mut formatter = definition::Formatter::new(
        &definitions.source,
        definitions.max_functions,
        vec!["namespace functions {".to_owned(), String::new()],
    );
    for (index, tool) in definitions.tools.iter().enumerate() {
        formatter.push_tool(index, tool.as_ref())?;
    }
    formatter.lines.push("} // namespace functions".to_owned());
    Ok(formatter.lines.join("\n"))
}
