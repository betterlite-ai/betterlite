use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::{self, SchemaNode};

#[derive(Debug, Deserialize)]
pub(super) struct Function {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<SchemaNode>,
}

#[derive(Debug)]
pub(super) enum Tool {
    Function(Box<Function>),
    Declarations(Vec<Function>),
}

// step one records each branch field's text; step two parses only the branch LiteLLM takes
#[derive(Debug, Deserialize)]
struct ToolFields {
    #[serde(default)]
    function: Option<Box<RawValue>>,
    #[serde(default)]
    function_declarations: Option<Box<RawValue>>,
    #[serde(default, rename = "functionDeclarations")]
    camel_declarations: Option<Box<RawValue>>,
    #[serde(default)]
    input_schema: Option<Box<RawValue>>,
    #[serde(default)]
    parameters: Option<Box<RawValue>>,
    #[serde(default)]
    name: Option<Box<RawValue>>,
    #[serde(default)]
    description: Option<Box<RawValue>>,
}

pub(super) fn parse_tool(index: usize, raw: &RawValue) -> Result<Option<Tool>, ToolFormatError> {
    // LiteLLM skips a tool entry that is not an object
    if !raw.get().trim_start().starts_with('{') {
        return Ok(None);
    }
    let fields: ToolFields = serde_json::from_str(raw.get())
        .map_err(|source| ToolFormatError::Object { index, source })?;
    if let Some(source) = object_field(fields.function.as_deref()) {
        let function = parse_branch(index, "function", source)?;
        return Ok(Some(Tool::Function(Box::new(function))));
    }
    let declarations = array_field(fields.function_declarations.as_deref())
        .or_else(|| array_field(fields.camel_declarations.as_deref()));
    if let Some(found) = declarations {
        let functions = declaration_functions(index, found)?;
        return Ok(Some(Tool::Declarations(functions)));
    }
    Ok(Some(Tool::Function(Box::new(inline_function(
        index, &fields,
    )?))))
}

fn object_field(field: Option<&RawValue>) -> Option<&RawValue> {
    let raw = field?;
    if !raw.get().trim_start().starts_with('{') {
        return None;
    }
    Some(raw)
}

fn array_field(field: Option<&RawValue>) -> Option<&RawValue> {
    let raw = field?;
    if !raw.get().trim_start().starts_with('[') {
        return None;
    }
    Some(raw)
}

fn declaration_functions(index: usize, raw: &RawValue) -> Result<Vec<Function>, ToolFormatError> {
    let entries: Vec<Box<RawValue>> = serde_json::from_str(raw.get())
        .map_err(|source| ToolFormatError::Object { index, source })?;
    let mut functions = Vec::new();
    for entry in &entries {
        // LiteLLM skips a declaration that is not an object
        if !entry.get().trim_start().starts_with('{') {
            continue;
        }
        functions.push(parse_branch(index, "functionDeclarations", entry)?);
    }
    Ok(functions)
}

fn inline_function(index: usize, fields: &ToolFields) -> Result<Function, ToolFormatError> {
    let parameters = inline_parameters(index, fields)?;
    Ok(Function {
        name: text_field(index, fields.name.as_deref(), "name")?,
        description: text_field(index, fields.description.as_deref(), "description")?,
        parameters,
    })
}

fn inline_parameters(
    index: usize,
    fields: &ToolFields,
) -> Result<Option<SchemaNode>, ToolFormatError> {
    if let Some(raw) = fields.input_schema.as_deref() {
        return Ok(Some(parse_branch(index, "input_schema", raw)?));
    }
    let Some(raw) = fields.parameters.as_deref() else {
        return Ok(None);
    };
    Ok(Some(parse_branch(index, "parameters", raw)?))
}

fn text_field(
    index: usize,
    value: Option<&RawValue>,
    field: &'static str,
) -> Result<Option<String>, ToolFormatError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    Ok(Some(parse_branch(index, field, raw)?))
}

fn parse_branch<T>(index: usize, field: &'static str, raw: &RawValue) -> Result<T, ToolFormatError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    serde_path_to_error::deserialize(&mut deserializer).map_err(|source| ToolFormatError::Shape {
        index,
        field,
        source,
    })
}

pub(super) fn format_tool(tool: &Tool, lines: &mut Vec<String>) -> Result<(), ToolFormatError> {
    match tool {
        Tool::Function(function) => lines.extend(format_function(function)?),
        Tool::Declarations(declarations) => {
            for declaration in declarations {
                lines.extend(format_function(declaration)?);
            }
        }
    }
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
    if node.property_map()?.is_none_or(IndexMap::is_empty) {
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
