use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

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
        if fields.get("type").and_then(Json::as_str) == Some("function")
            && fields.contains_key("function")
        {
            converted.push(Json::Object(function_tool(fields)?));
        } else if let Some(schema) = fields.get("input_schema") {
            let mut kept = fields.clone();
            kept.insert("input_schema".to_owned(), clean_input_schema(schema));
            converted.push(Json::Object(kept));
        } else {
            converted.push(tool.clone());
        }
    }
    Ok(converted)
}

fn function_tool(fields: &Object) -> Result<Object, ToolError> {
    let function = fields
        .get("function")
        .and_then(Json::as_object)
        .ok_or(ToolError::FunctionNotObject)?;
    let mut converted = Object::from_iter([(
        "name".to_owned(),
        function
            .get("name")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    )]);
    if let Some(description) = function.get("description") {
        converted.insert("description".to_owned(), description.clone());
    }
    let schema = function
        .get("parameters")
        .map_or_else(default_schema, clean_input_schema);
    converted.insert("input_schema".to_owned(), schema);
    Ok(converted)
}

fn default_schema() -> Json {
    Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("object".to_owned())),
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
    cleaned.shift_remove("$schema");
    Json::Object(cleaned)
}

pub fn transform_tool_choice(tool_choice: &Json) -> Result<Json, ToolError> {
    match tool_choice {
        Json::Text(name) => Ok(choice_kind(name)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Ok(choice_kind("auto")),
        Json::Object(fields) => function_choice(tool_choice, fields),
    }
}

fn function_choice(tool_choice: &Json, fields: &Object) -> Result<Json, ToolError> {
    if fields.get("type").and_then(Json::as_str) != Some("function") {
        return Ok(tool_choice.clone());
    }
    let name = match fields.get("function") {
        None | Some(Json::Null) => Json::Text(String::new()),
        Some(Json::Object(function)) => function
            .get("name")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
        Some(_) => return Err(ToolError::ChoiceFunctionNotObject),
    };
    Ok(Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("tool".to_owned())),
        ("name".to_owned(), name),
    ])))
}

fn choice_kind(kind: &str) -> Json {
    let name = if kind == "required" {
        "any"
    } else if kind == "none" {
        "none"
    } else {
        "auto"
    };
    Json::Object(Object::from_iter([(
        "type".to_owned(),
        Json::Text(name.to_owned()),
    )]))
}
