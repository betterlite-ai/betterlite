use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::expand::unpack_defs;
use serde::{Deserialize, Serialize};

use super::{SchemaError, check_bounds, filter_anthropic_output_schema};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ResponseFormat {
    #[serde(default, rename = "type")]
    pub type_name: Field,
    #[serde(default)]
    pub response_schema: Field,
    #[serde(default)]
    pub json_schema: Field,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(untagged)]
pub enum Field {
    Value(Json),
    #[default]
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputFormat {
    JsonSchema { schema: Json },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    pub name: String,
    pub input_schema: Json,
}

pub fn map_output(value: Option<&ResponseFormat>) -> Result<Option<OutputFormat>, SchemaError> {
    let Some(schema) = schema_from_response(value)? else {
        return Ok(None);
    };
    check_bounds(schema)?;
    let expanded = expand_definitions(schema.clone())?;
    Ok(Some(OutputFormat::JsonSchema {
        schema: filter_anthropic_output_schema(expanded)?,
    }))
}

fn schema_from_response(value: Option<&ResponseFormat>) -> Result<Option<&Json>, SchemaError> {
    let Some(format) = value else {
        return Ok(None);
    };
    let schema = match &format.response_schema {
        Field::Value(schema) => Some(schema),
        Field::Missing => match &format.json_schema {
            Field::Missing => None,
            Field::Value(nested) => Some(schema_field(nested)?),
        },
    };
    Ok(schema.filter(|candidate| !matches!(candidate, Json::Null)))
}

fn schema_field(nested: &Json) -> Result<&Json, SchemaError> {
    match nested {
        Json::Object(fields) => fields
            .get("schema")
            .ok_or(SchemaError::Invalid("json_schema")),
        _ => Err(SchemaError::Invalid("json_schema")),
    }
}

fn expand_definitions(schema: Json) -> Result<Json, SchemaError> {
    let Json::Object(mut fields) = schema else {
        return Err(SchemaError::Invalid("schema"));
    };
    let defs = fields.shift_remove("$defs");
    let legacy = fields.shift_remove("definitions");
    match defs.or(legacy).filter(Json::is_truthy) {
        Some(Json::Object(definitions)) => Ok(Json::Object(unpack_defs(
            fields,
            &definitions,
            Some(10_000_000),
        )?)),
        Some(_) => Err(SchemaError::Invalid("definitions")),
        None => Ok(Json::Object(fields)),
    }
}

pub fn map_tool(value: Option<&ResponseFormat>) -> Result<Option<Tool>, SchemaError> {
    match value.map(|format| &format.type_name) {
        None => Ok(None),
        Some(Field::Missing) => Err(SchemaError::MissingType),
        Some(Field::Value(Json::Text(kind))) if kind == "text" => Ok(None),
        Some(Field::Value(_)) => schema_from_response(value)?
            .map(|schema| json_tool(Some(schema)))
            .transpose(),
    }
}

pub fn json_tool(schema: Option<&Json>) -> Result<Tool, SchemaError> {
    if let Some(provided) = schema {
        check_bounds(provided)?;
    }
    let mut input_schema =
        Object::from_iter([(String::from("type"), Json::Text(String::from("object")))]);
    match schema {
        None | Some(Json::Null) => {
            input_schema.insert(String::from("additionalProperties"), Json::Bool(true));
            input_schema.insert(String::from("properties"), Json::Object(Object::new()));
        }
        Some(Json::Object(fields)) => {
            input_schema.extend(fields.clone());
        }
        Some(_) => return Err(SchemaError::Invalid("schema")),
    }
    Ok(Tool {
        name: "json_tool_call".to_owned(),
        input_schema: Json::Object(input_schema),
    })
}
