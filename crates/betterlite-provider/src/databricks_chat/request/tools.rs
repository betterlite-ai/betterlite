use betterlite_anthropic::output_schema::mapping::{ResponseFormat, map_tool};
use betterlite_anthropic::params::tools::RESPONSE_FORMAT_TOOL_NAME;
use betterlite_anthropic::tools::map_tools;
use betterlite_prompts::arguments::json::{Json, Object};

use super::CLAUDE;
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::wire::{Params, encode_value};

// LiteLLM's _map_openai_to_dbrx_tool: a non-claude model passes tools through untouched
pub(crate) fn map_tools_for_model(model: &str, value: &Json) -> Result<Json, DatabricksChatError> {
    if !model.contains(CLAUDE) {
        return Ok(value.clone());
    }
    let Json::List(tools) = value else {
        return Err(DatabricksChatError::Shape {
            message: "the tools value must be a list".to_owned(),
        });
    };
    let mapping = map_tools(tools)?;
    let mut converted = Vec::with_capacity(mapping.tools.len());
    for tool in &mapping.tools {
        let encoded_tool = encode_value(tool)?;
        converted.push(convert_anthropic_tool(&encoded_tool)?);
    }
    Ok(Json::List(converted))
}

// convert_anthropic_tool_to_databricks_tool: name and input_schema move under a function key
pub(crate) fn convert_anthropic_tool(value: &Json) -> Result<Json, DatabricksChatError> {
    let Some(fields) = value.as_object() else {
        return Err(DatabricksChatError::Shape {
            message: "a mapped Anthropic tool must be an object".to_owned(),
        });
    };
    let Some(name) = fields.get("name").and_then(Json::as_str) else {
        return Err(DatabricksChatError::Shape {
            message: "a mapped Anthropic tool is missing its name".to_owned(),
        });
    };
    let parameters = match fields.get("input_schema") {
        Some(schema) if schema.is_truthy() => schema.clone(),
        None
        | Some(Json::Null)
        | Some(Json::Bool(_))
        | Some(Json::Number(_))
        | Some(Json::Text(_))
        | Some(Json::List(_))
        | Some(Json::Object(_)) => Json::Object(Object::new()),
    };
    Ok(build_tool(name, parameters, fields.get("description")))
}

// A Databricks tool is {"type": "function", "function": {name, parameters, description?}}
pub(crate) fn build_tool(name: &str, parameters: Json, description: Option<&Json>) -> Json {
    let mut function = Object::new();
    function.insert("name".to_owned(), Json::Text(name.to_owned()));
    function.insert("parameters".to_owned(), parameters);
    if let Some(description) = description
        && !matches!(description, Json::Null)
    {
        function.insert("description".to_owned(), description.clone());
    }
    let mut tool = Object::new();
    tool.insert("type".to_owned(), Json::Text("function".to_owned()));
    tool.insert("function".to_owned(), Json::Object(function));
    Json::Object(tool)
}

// map_response_format_to_databricks_tool: text formats map to no tool, the rest to json_tool_call
pub(crate) fn response_format_tool(value: &Json) -> Result<Option<Json>, DatabricksChatError> {
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_value(value).map_err(|source| DatabricksChatError::Encode(source))?;
    let format: ResponseFormat =
        serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)?;
    let Some(tool) = map_tool(Some(&format))? else {
        return Ok(None);
    };
    let parameters = if tool.input_schema.is_truthy() {
        tool.input_schema.clone()
    } else {
        Json::Object(Object::new())
    };
    let name = RESPONSE_FORMAT_TOOL_NAME;
    Ok(Some(build_tool(name, parameters, None)))
}

// _add_tools_to_optional_params: append to the existing list, create it when absent
pub(crate) fn append_wire_tool(params: &mut Params, tool: Json) -> Result<(), DatabricksChatError> {
    if let Some(existing) = params.get_mut("tools") {
        match existing {
            Json::List(tools) => {
                tools.push(tool);
                return Ok(());
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                return Err(DatabricksChatError::Shape {
                    message: "the existing tools value is not a list".to_owned(),
                });
            }
        }
    }
    params.insert("tools".to_owned(), Json::List(vec![tool]));
    Ok(())
}

// The json-mode tool_choice names the response_format tool without arguments
pub(crate) fn json_tool_choice() -> Json {
    let mut function = Object::new();
    function.insert(
        "name".to_owned(),
        Json::Text(RESPONSE_FORMAT_TOOL_NAME.to_owned()),
    );
    let mut choice = Object::new();
    choice.insert("type".to_owned(), Json::Text("function".to_owned()));
    choice.insert("function".to_owned(), Json::Object(function));
    Json::Object(choice)
}
