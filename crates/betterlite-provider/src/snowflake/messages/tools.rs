use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

const TYPE: &str = "type";
const FUNCTION: &str = "function";
const NAME: &str = "name";
const DESCRIPTION: &str = "description";
const PARAMETERS: &str = "parameters";
const INPUT_SCHEMA: &str = "input_schema";
const SCHEMA_KEY: &str = "$schema";

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tools must be a list of objects")]
    NotObject,
    #[error("a function tool's function must be an object")]
    FunctionNotObject,
    #[error("tool_choice.function must be an object")]
    ChoiceFunctionNotObject,
}

pub fn transform_tools(tools: &[Json]) -> Result<Vec<Json>, ToolError> {
    let mut converted = Vec::with_capacity(tools.len());
    for tool in tools {
        let fields = tool.as_object().ok_or(ToolError::NotObject)?;
        if fields.get(TYPE).and_then(Json::as_str) == Some(FUNCTION)
            && fields.contains_key(FUNCTION)
        {
            converted.push(Json::Object(function_tool(fields)?));
        } else if let Some(schema) = fields.get(INPUT_SCHEMA) {
            let mut kept = fields.clone();
            kept.insert(INPUT_SCHEMA.to_owned(), clean_input_schema(schema));
            converted.push(Json::Object(kept));
        } else {
            converted.push(tool.clone());
        }
    }
    Ok(converted)
}

fn function_tool(fields: &Object) -> Result<Object, ToolError> {
    let function = fields
        .get(FUNCTION)
        .and_then(Json::as_object)
        .ok_or(ToolError::FunctionNotObject)?;
    let mut converted = Object::from_iter([(
        NAME.to_owned(),
        function
            .get(NAME)
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    )]);
    if let Some(description) = function.get(DESCRIPTION) {
        converted.insert(DESCRIPTION.to_owned(), description.clone());
    }
    let schema = function
        .get(PARAMETERS)
        .map_or_else(default_schema, clean_input_schema);
    converted.insert(INPUT_SCHEMA.to_owned(), schema);
    Ok(converted)
}

fn default_schema() -> Json {
    Json::Object(Object::from_iter([
        (TYPE.to_owned(), Json::Text("object".to_owned())),
        ("properties".to_owned(), Json::Object(Object::new())),
    ]))
}

// LiteLLM drops only a top-level "$schema"; every other key and its order stay
#[must_use]
pub fn clean_input_schema(schema: &Json) -> Json {
    let Some(fields) = schema.as_object() else {
        return schema.clone();
    };
    let mut cleaned = fields.clone();
    cleaned.shift_remove(SCHEMA_KEY);
    Json::Object(cleaned)
}

pub fn transform_tool_choice(tool_choice: &Json) -> Result<Json, ToolError> {
    match tool_choice {
        Json::Text(name) => Ok(choice_kind(name)),
        Json::Object(fields) => {
            if fields.get(TYPE).and_then(Json::as_str) == Some(FUNCTION) {
                let name = match fields.get(FUNCTION) {
                    None | Some(Json::Null) => Json::Text(String::new()),
                    Some(Json::Object(function)) => function
                        .get(NAME)
                        .cloned()
                        .unwrap_or_else(|| Json::Text(String::new())),
                    Some(_) => return Err(ToolError::ChoiceFunctionNotObject),
                };
                return Ok(Json::Object(Object::from_iter([
                    (TYPE.to_owned(), Json::Text("tool".to_owned())),
                    (NAME.to_owned(), name),
                ])));
            }
            Ok(tool_choice.clone())
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Ok(choice_kind("auto")),
    }
}

fn choice_kind(kind: &str) -> Json {
    let name = match kind {
        "auto" => "auto",
        "required" => "any",
        "none" => "none",
        _ => "auto",
    };
    Json::Object(Object::from_iter([(
        TYPE.to_owned(),
        Json::Text(name.to_owned()),
    )]))
}
