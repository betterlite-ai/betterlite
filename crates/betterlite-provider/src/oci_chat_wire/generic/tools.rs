use betterlite_openai::chat::Tool;

use crate::oci_chat_wire::tool_schema::resolved_tool_schema;
use crate::oci_chat_wire::types::oci::{OciFunctionType, OciToolDefinition};
use crate::oci_common::OciError;

pub fn adapt_generic_tools(tools: &[Tool]) -> Result<Vec<OciToolDefinition>, OciError> {
    let mut definitions = Vec::with_capacity(tools.len());
    for tool in tools {
        if tool.kind != "function" {
            return Err(OciError {
                status: 400,
                message: String::from("OCI only supports function tools"),
            });
        }
        let Some(function) = &tool.function else {
            return Err(OciError {
                status: 400,
                message: String::from("Tool `function` must be a dictionary"),
            });
        };
        definitions.push(OciToolDefinition {
            kind: OciFunctionType::Function,
            name: Some(function.name.clone()),
            description: Some(function.description.clone().unwrap_or_default()),
            parameters: Some(resolved_tool_schema(Some(function))?),
        });
    }
    Ok(definitions)
}
