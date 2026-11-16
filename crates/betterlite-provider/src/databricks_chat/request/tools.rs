use betterlite_anthropic::output_schema::mapping::{ResponseFormat, map_tool};
use betterlite_anthropic::params::tools::RESPONSE_FORMAT_TOOL_NAME;
use betterlite_anthropic::tools::map_tools;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};

use super::CLAUDE;
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::wire::{Params, convert};

// LiteLLM's _map_openai_to_dbrx_tool: a non-claude model passes tools through untouched
pub(crate) fn map_tools_for_model(model: &str, value: &Json) -> Result<Json, DatabricksChatError> {
    if !model.contains(CLAUDE) {
        return Ok(value.clone());
    }
    let Json::List(tools) = value else {
        return Err(DatabricksChatError::Shape {
            message: String::from("the tools value must be a list"),
        });
    };
    let mut converted = Vec::with_capacity(tools.len());
    for tool in &map_tools(tools)?.tools {
        converted.push(convert_anthropic_tool(&convert(tool)?)?);
    }
    Ok(Json::List(converted))
}

// convert_anthropic_tool_to_databricks_tool: name and input_schema move under a function key
pub(crate) fn convert_anthropic_tool(value: &Json) -> Result<Json, DatabricksChatError> {
    let Some(fields) = value.as_object() else {
        return Err(DatabricksChatError::Shape {
            message: String::from("a mapped Anthropic tool must be an object"),
        });
    };
    let Some(name) = fields.get("name").and_then(Json::as_str) else {
        return Err(DatabricksChatError::Shape {
            message: String::from("a mapped Anthropic tool is missing its name"),
        });
    };
    Ok(build_tool(
        name,
        fields.get("input_schema"),
        fields.get("description"),
    ))
}

// A Databricks tool is {"type": "function", "function": {name, parameters, description?}}
pub(crate) fn build_tool(name: &str, schema: Option<&Json>, description: Option<&Json>) -> Json {
    let parameters = schema
        .filter(|given| given.is_truthy())
        .cloned()
        .unwrap_or_else(|| Json::Object(Object::new()));
    let mut function = Object::from_iter([
        (String::from("name"), Json::Text(name.to_owned())),
        (String::from("parameters"), parameters),
    ]);
    if let Some(text) = description.filter(|given| !matches!(given, Json::Null)) {
        function.insert(String::from("description"), text.clone());
    }
    Json::Object(Object::from_iter([
        (String::from("type"), Json::Text(String::from("function"))),
        (String::from("function"), Json::Object(function)),
    ]))
}

// map_response_format_to_databricks_tool: text formats map to no tool, the rest to json_tool_call
pub(crate) fn response_format_tool(value: &Json) -> Result<Option<Json>, DatabricksChatError> {
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let format: ResponseFormat = convert(value)?;
    let Some(tool) = map_tool(Some(&format))? else {
        return Ok(None);
    };
    Ok(Some(build_tool(
        RESPONSE_FORMAT_TOOL_NAME,
        Some(&tool.input_schema),
        None,
    )))
}

// _add_tools_to_optional_params: append to the existing list, create it when absent
pub(crate) fn append_wire_tool(
    params: &mut Params,
    tool: DecisionJson,
) -> Result<(), DatabricksChatError> {
    match params.get_mut("tools") {
        Some(DecisionJson::List(tools)) => tools.push(tool),
        None => {
            params.insert(String::from("tools"), DecisionJson::List(vec![tool]));
        }
        Some(
            DecisionJson::Null
            | DecisionJson::Bool(_)
            | DecisionJson::Number(_)
            | DecisionJson::Text(_)
            | DecisionJson::Object(_),
        ) => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the existing tools value is not a list"),
            });
        }
    }
    Ok(())
}

// The json-mode tool_choice names the response_format tool without arguments
pub(crate) fn json_tool_choice() -> Json {
    Json::Object(Object::from_iter([
        (String::from("type"), Json::Text(String::from("function"))),
        (
            String::from("function"),
            Json::Object(Object::from_iter([(
                String::from("name"),
                Json::Text(RESPONSE_FORMAT_TOOL_NAME.to_owned()),
            )])),
        ),
    ]))
}
