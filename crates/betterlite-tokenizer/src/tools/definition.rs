use serde::Deserialize;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::{self, SchemaNode};

#[derive(Debug, Deserialize)]
pub(super) struct Function<'raw> {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(borrow, default)]
    parameters: Option<SchemaNode<'raw>>,
}

// step one records each branch field's text; step two parses only the branch LiteLLM takes
#[derive(Debug, Deserialize)]
struct ToolFields<'raw> {
    #[serde(borrow, default)]
    function: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    function_declarations: Option<&'raw RawValue>,
    #[serde(rename = "functionDeclarations", borrow, default)]
    camel_declarations: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    input_schema: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    parameters: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    name: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    description: Option<&'raw RawValue>,
}

pub(super) fn format_tool(
    index: usize,
    raw: &RawValue,
    used: usize,
    limit: usize,
    lines: &mut Vec<String>,
) -> Result<usize, ToolFormatError> {
    // LiteLLM skips a tool entry that is not an object
    if !raw.get().trim_start().starts_with('{') {
        return Ok(used);
    }
    let fields: ToolFields<'_> = serde_json::from_str(raw.get())
        .map_err(|source| ToolFormatError::Object { index, source })?;
    if let Some(source) = object_field(fields.function) {
        let total = charge(used, 1, limit)?;
        let function = parse_branch(index, "function", source)?;
        lines.extend(format_function(&function)?);
        return Ok(total);
    }
    let declarations = array_field(fields.function_declarations)
        .or_else(|| array_field(fields.camel_declarations));
    if let Some(source) = declarations {
        let entries: Vec<&RawValue> =
            serde_json::from_str(source.get()).map_err(|failure| ToolFormatError::Object {
                index,
                source: failure,
            })?;
        let total = charge(used, entries.len(), limit)?;
        for entry in entries {
            // LiteLLM skips a declaration that is not an object
            if !entry.get().trim_start().starts_with('{') {
                continue;
            }
            let function = parse_branch(index, "functionDeclarations", entry)?;
            lines.extend(format_function(&function)?);
        }
        return Ok(total);
    }
    let total = charge(used, 1, limit)?;
    let function = inline_function(index, &fields)?;
    lines.extend(format_function(&function)?);
    Ok(total)
}

const fn charge(used: usize, count: usize, limit: usize) -> Result<usize, ToolFormatError> {
    let total = used.saturating_add(count);
    if total > limit {
        return Err(ToolFormatError::TooMany {
            count: total,
            limit,
        });
    }
    Ok(total)
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

fn inline_function<'raw>(
    index: usize,
    fields: &ToolFields<'raw>,
) -> Result<Function<'raw>, ToolFormatError> {
    let parameters = inline_parameters(index, fields)?;
    Ok(Function {
        name: text_field(index, fields.name, "name")?,
        description: text_field(index, fields.description, "description")?,
        parameters,
    })
}

fn inline_parameters<'raw>(
    index: usize,
    fields: &ToolFields<'raw>,
) -> Result<Option<SchemaNode<'raw>>, ToolFormatError> {
    if let Some(raw) = fields.input_schema {
        return Ok(Some(parse_branch(index, "input_schema", raw)?));
    }
    let Some(raw) = fields.parameters else {
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

fn parse_branch<'de, T>(
    index: usize,
    field: &'static str,
    raw: &'de RawValue,
) -> Result<T, ToolFormatError>
where
    T: Deserialize<'de>,
{
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    serde_path_to_error::deserialize(&mut deserializer).map_err(|source| ToolFormatError::Shape {
        index,
        field,
        source,
    })
}

fn format_function(function: &Function<'_>) -> Result<Vec<String>, ToolFormatError> {
    format_definition(
        function.name.as_deref(),
        function.description.as_deref(),
        function.parameters.as_ref(),
    )
}

fn format_definition(
    function_name: Option<&str>,
    description: Option<&str>,
    parameters: Option<&SchemaNode<'_>>,
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
    let rendered = schema::object_parameters(node, 0, 0)?;
    if rendered.is_empty() {
        return Ok(single_parameter_list(lines, name));
    }
    lines.push(format!("type {name} = (_: {{"));
    lines.push(rendered);
    lines.push("}) => any;".to_owned());
    lines.push(String::new());
    Ok(lines)
}

fn single_parameter_list(mut lines: Vec<String>, name: &str) -> Vec<String> {
    lines.push(format!("type {name} = () => any;"));
    lines.push(String::new());
    lines
}
