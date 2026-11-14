use betterlite_context::non_default_params::schema_cleanup::{
    remove_additional_properties, remove_strict_from_schema,
};
use betterlite_models::controls::json_value::JsonValue;
use serde_json::Value;

use super::error::HostedVllmChatError;
use crate::openai_base_config::Object;

const CUSTOM: &str = "custom";
const DESCRIPTION: &str = "description";
const FUNCTION: &str = "function";
const INPUT: &str = "input";
const INPUT_DESCRIPTION: &str = "Raw tool input payload.";
const INPUT_SCHEMA: &str = "input_schema";
const NAME: &str = "name";
const OBJECT: &str = "object";
const PARAMETERS: &str = "parameters";
const PROPERTIES: &str = "properties";
const REQUIRED: &str = "required";
const STRING: &str = "string";
const TOOL_TYPE: &str = "type";

#[derive(Clone, Copy)]
enum ToolField {
    Name,
    Description,
    InputSchema,
}

impl ToolField {
    const fn key(self) -> &'static str {
        match self {
            Self::Name => NAME,
            Self::Description => DESCRIPTION,
            Self::InputSchema => INPUT_SCHEMA,
        }
    }

    const fn type_error(self) -> HostedVllmChatError {
        match self {
            Self::Name => HostedVllmChatError::CustomToolString { field: NAME },
            Self::Description => HostedVllmChatError::CustomToolString { field: DESCRIPTION },
            Self::InputSchema => HostedVllmChatError::CustomToolSchema,
        }
    }

    fn accepts(self, given: &Value) -> bool {
        match self {
            Self::Name | Self::Description => given.is_string(),
            Self::InputSchema => given.is_object(),
        }
    }

    fn is_empty(self, given: &Value) -> bool {
        match self {
            Self::Name => given.as_str().is_some_and(str::is_empty),
            Self::Description => given.as_str() == Some(""),
            Self::InputSchema => given.as_object().is_some_and(Object::is_empty),
        }
    }
}

// the last source is the fallback's final operand, so its empty description or schema is used as given
fn tool_field_value<'source>(
    sources: &[&'source Object],
    field: ToolField,
) -> Result<Option<&'source Value>, HostedVllmChatError> {
    for (position, source) in sources.iter().enumerate() {
        let Some(given) = source.get(field.key()) else {
            continue;
        };
        if given.is_null() {
            continue;
        }
        if !field.accepts(given) {
            return Err(field.type_error());
        }
        let skipped = field.is_empty(given)
            && (position.saturating_add(1) < sources.len() || matches!(field, ToolField::Name));
        if skipped {
            continue;
        }
        return Ok(Some(given));
    }
    Ok(None)
}

fn tool_name_value(sources: &[&Object], index: usize) -> Result<String, HostedVllmChatError> {
    Ok(tool_field_value(sources, ToolField::Name)?
        .and_then(Value::as_str)
        .map_or_else(|| format!("custom_tool_{index}"), str::to_owned))
}

fn tool_description_value(sources: &[&Object]) -> Result<Option<String>, HostedVllmChatError> {
    Ok(tool_field_value(sources, ToolField::Description)?
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn default_input_schema() -> Value {
    let input = Object::from_iter([
        (String::from(TOOL_TYPE), Value::String(String::from(STRING))),
        (
            String::from(DESCRIPTION),
            Value::String(String::from(INPUT_DESCRIPTION)),
        ),
    ]);
    let properties = Object::from_iter([(String::from(INPUT), Value::Object(input))]);
    Value::Object(Object::from_iter([
        (String::from(TOOL_TYPE), Value::String(String::from(OBJECT))),
        (String::from(PROPERTIES), Value::Object(properties)),
        (
            String::from(REQUIRED),
            Value::Array(vec![Value::String(String::from(INPUT))]),
        ),
    ]))
}

fn convert_custom_tool(fields: &Object, index: usize) -> Result<Value, HostedVllmChatError> {
    let custom_field = match fields.get(CUSTOM) {
        None | Some(Value::Null) => None,
        Some(Value::Object(custom)) => Some(custom),
        Some(_) => return Err(HostedVllmChatError::CustomToolObject),
    };
    let mut sources: Vec<&Object> = Vec::with_capacity(2);
    if let Some(custom) = custom_field {
        sources.push(custom);
    }
    sources.push(fields);
    let name = tool_name_value(&sources, index)?;
    let description = tool_description_value(&sources)?;
    let parameters = tool_field_value(&sources, ToolField::InputSchema)?
        .map_or_else(default_input_schema, Clone::clone);
    let mut function = Object::new();
    function.insert(String::from(NAME), Value::String(name));
    function.insert(String::from(PARAMETERS), parameters);
    if let Some(text) = description {
        function.insert(String::from(DESCRIPTION), Value::String(text));
    }
    Ok(Value::Object(Object::from_iter([
        (
            String::from(TOOL_TYPE),
            Value::String(String::from(FUNCTION)),
        ),
        (String::from(FUNCTION), Value::Object(function)),
    ])))
}

fn convert_tools(items: Vec<Value>) -> Result<Vec<Value>, HostedVllmChatError> {
    let mut converted = Vec::with_capacity(items.len());
    for (index, tool) in items.into_iter().enumerate() {
        let Value::Object(fields) = tool else {
            return Err(HostedVllmChatError::ToolNotObject { index });
        };
        if fields.get(TOOL_TYPE).and_then(Value::as_str) != Some(CUSTOM) {
            converted.push(Value::Object(fields));
            continue;
        }
        converted.push(convert_custom_tool(&fields, index)?);
    }
    Ok(converted)
}

pub(crate) fn clean_and_convert(tools: Value) -> Result<Value, HostedVllmChatError> {
    let mut schema = serde_json::from_value::<JsonValue>(tools)
        .map_err(|source| HostedVllmChatError::Json { source })?;
    remove_additional_properties(&mut schema);
    remove_strict_from_schema(&mut schema);
    let cleaned =
        serde_json::to_value(schema).map_err(|source| HostedVllmChatError::Json { source })?;
    match cleaned {
        Value::Array(items) => Ok(Value::Array(convert_tools(items)?)),
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_)) => Ok(other),
    }
}
