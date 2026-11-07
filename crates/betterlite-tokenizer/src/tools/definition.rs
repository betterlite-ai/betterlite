use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::{self, SchemaNode};

#[derive(Debug, Deserialize)]
pub(super) struct Tool {
    #[serde(default)]
    function: Option<Function>,
    #[serde(default)]
    function_declarations: Option<Vec<Function>>,
    // LiteLLM also accepts Gemini's camelCase spelling and prefers the snake_case list when both appear
    #[serde(rename = "functionDeclarations", default)]
    camel_declarations: Option<Vec<Function>>,
    #[serde(default)]
    input_schema: Option<SchemaNode>,
    #[serde(default)]
    parameters: Option<SchemaNode>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Function {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<SchemaNode>,
}

pub(super) fn parse_tool(index: usize, raw: &RawValue) -> Result<Option<Tool>, ToolFormatError> {
    // LiteLLM skips a tool entry that is not an object
    if !raw.get().trim_start().starts_with('{') {
        return Ok(None);
    }
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let tool = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| ToolFormatError::Shape { index, source })?;
    Ok(Some(tool))
}

pub(super) fn format_tool(tool: &Tool, lines: &mut Vec<String>) -> Result<(), ToolFormatError> {
    if let Some(function) = &tool.function {
        lines.extend(format_function(function)?);
        return Ok(());
    }
    let declarations = tool
        .function_declarations
        .as_ref()
        .or(tool.camel_declarations.as_ref());
    if let Some(found) = declarations {
        for declaration in found {
            lines.extend(format_function(declaration)?);
        }
        return Ok(());
    }
    let parameters = tool.input_schema.as_ref().or(tool.parameters.as_ref());
    lines.extend(format_definition(
        tool.name.as_deref(),
        tool.description.as_deref(),
        parameters,
    )?);
    Ok(())
}

fn format_function(function: &Function) -> Result<Vec<String>, ToolFormatError> {
    format_definition(
        function.name.as_deref(),
        function.description.as_deref(),
        function.parameters.as_ref(),
    )
}

fn format_definition(
    function_name: Option<&str>,
    description: Option<&str>,
    parameters: Option<&SchemaNode>,
) -> Result<Vec<String>, ToolFormatError> {
    // a function with no name has no type declaration to emit, as in LiteLLM
    let Some(name) = function_name.filter(|candidate| !candidate.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    if let Some(text) = description {
        lines.push(format!("// {text}"));
    }
    let Some(node) = parameters else {
        return Ok(single_parameter_list(lines, name));
    };
    if node.properties.as_ref().is_none_or(IndexMap::is_empty) {
        return Ok(single_parameter_list(lines, name));
    }
    lines.push(format!("type {name} = (_: {{"));
    lines.push(schema::object_parameters(node, 0)?);
    lines.push("}) => any;".to_owned());
    lines.push(String::new());
    Ok(lines)
}

fn single_parameter_list(mut lines: Vec<String>, name: &str) -> Vec<String> {
    lines.push(format!("type {name} = () => any;"));
    lines.push(String::new());
    lines
}
