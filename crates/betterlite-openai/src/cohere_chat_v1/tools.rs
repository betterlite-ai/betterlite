use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;

use super::error::CohereV1ConfigError;

#[derive(Deserialize)]
struct OpenAiTool {
    function: OpenAiFunction,
}

#[derive(Deserialize)]
struct OpenAiFunction {
    name: Json,
    // LiteLLM reads the description with [] instead of .get, so a missing one fails the request
    description: Json,
    parameters: OpenAiParameters,
}

#[derive(Deserialize)]
struct OpenAiParameters {
    properties: IndexMap<String, OpenAiProperty>,
    #[serde(default)]
    required: Option<Json>,
}

#[derive(Deserialize)]
struct OpenAiProperty {
    #[serde(default)]
    description: Option<Json>,
    #[serde(rename = "type", default)]
    kind: Option<Json>,
}

pub fn cohere_tools(tools: &Json) -> Result<Json, CohereV1ConfigError> {
    let entries = match tools {
        // LiteLLM turns a null tools value into its own empty list
        Json::Null => return Ok(Json::List(Vec::new())),
        Json::List(entries) => entries,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(CohereV1ConfigError::ToolsNotList);
        }
    };
    let translated = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| cohere_tool(index, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::List(translated))
}

fn cohere_tool(index: usize, entry: &Json) -> Result<Json, CohereV1ConfigError> {
    let tool = decode_tool(index, entry)?;
    let required = required_of(index, tool.function.parameters.required)?;
    let mut cohere_tool = Object::new();
    cohere_tool.insert("name".to_owned(), tool.function.name);
    cohere_tool.insert("description".to_owned(), tool.function.description);
    cohere_tool.insert(
        "parameter_definitions".to_owned(),
        definitions_of(tool.function.parameters.properties, &required),
    );
    Ok(Json::Object(cohere_tool))
}

fn decode_tool(index: usize, entry: &Json) -> Result<OpenAiTool, CohereV1ConfigError> {
    let encoded = serde_json::to_value(entry)
        .map_err(|source| CohereV1ConfigError::ToolShape { index, source })?;
    serde_json::from_value(encoded)
        .map_err(|source| CohereV1ConfigError::ToolShape { index, source })
}

fn required_of(index: usize, required: Option<Json>) -> Result<Vec<Json>, CohereV1ConfigError> {
    match required {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(names)) => Ok(names),
        Some(_) => Err(CohereV1ConfigError::Tool {
            index,
            detail: "parameters.required is not a list",
        }),
    }
}

fn definitions_of(properties: IndexMap<String, OpenAiProperty>, required: &[Json]) -> Json {
    let mut definitions = Object::new();
    for (property_name, property) in properties {
        let mut definition = Object::new();
        definition.insert(
            "description".to_owned(),
            property
                .description
                .unwrap_or_else(|| Json::Text(String::new())),
        );
        definition.insert(
            "type".to_owned(),
            property.kind.unwrap_or_else(|| Json::Text(String::new())),
        );
        let is_required = required
            .iter()
            .any(|required_name| required_name.as_str() == Some(property_name.as_str()));
        definition.insert("required".to_owned(), Json::Bool(is_required));
        definitions.insert(property_name, Json::Object(definition));
    }
    Json::Object(definitions)
}
