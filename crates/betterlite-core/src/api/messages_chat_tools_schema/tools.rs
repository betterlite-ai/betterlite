use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::messages_content_primitives::markers::copy_cache_control;
use crate::api::messages_content_primitives::tools::{
    chat_tool_param, is_openai_function_tool, is_provider_native_tool_dict, truncate_tool_name,
};

const HOSTED_TOOL_PREFIXES: [&str; 7] = [
    "web_search",
    "bash",
    "text_editor",
    "code_execution",
    "web_fetch",
    "memory",
    "tool_search_tool",
];
const TOOL_SCHEMA_KEYS: [&str; 7] = [
    "name",
    "input_schema",
    "description",
    "cache_control",
    "strict",
    "type",
    "eager_input_streaming",
];

pub type ToolNameMapping = IndexMap<String, String>;

#[derive(Debug)]
pub struct ToolsTranslation {
    pub tools: Vec<Json>,
    pub name_mapping: ToolNameMapping,
}

#[derive(Debug, Error)]
pub enum ToolsError {
    #[error("tools is not a list")]
    NotAList,
    #[error("tool {index} is not a JSON object")]
    NotAnObject { index: usize },
    #[error("tool {index} has a type that is not a string")]
    TypeNotText { index: usize },
    #[error("tool {index} has a name that is not a string or a number")]
    NameNotText { index: usize },
    #[error("tool {index} has an input_schema that is not an object")]
    SchemaNotObject { index: usize },
    #[error("the target model is not a string")]
    ModelNotText,
    #[error(transparent)]
    EagerInput(#[from] FeatureFlagsError),
}

pub fn tools_to_openai(
    tools: &[Json],
    model: Option<&str>,
) -> Result<ToolsTranslation, ToolsError> {
    let mut translated = Vec::with_capacity(tools.len());
    let mut name_mapping = ToolNameMapping::new();
    for (index, tool) in tools.iter().enumerate() {
        translated.push(translate_tool(tool, index, model, &mut name_mapping)?);
    }
    Ok(ToolsTranslation {
        tools: translated,
        name_mapping,
    })
}

fn translate_tool(
    tool: &Json,
    index: usize,
    model: Option<&str>,
    name_mapping: &mut ToolNameMapping,
) -> Result<Json, ToolsError> {
    let Some(fields) = tool.as_object() else {
        return Err(ToolsError::NotAnObject { index });
    };
    let tool_type = match fields.get("type") {
        None => "",
        Some(value) => value.as_str().ok_or(ToolsError::TypeNotText { index })?,
    };
    let passed_through = HOSTED_TOOL_PREFIXES
        .iter()
        .any(|prefix| tool_type.starts_with(prefix))
        || is_openai_function_tool(fields)
        || is_provider_native_tool_dict(fields);
    if passed_through {
        return Ok(tool.clone());
    }
    let original_name = declared_name(fields, index)?;
    let truncated_name = truncate_tool_name(&original_name);
    if truncated_name != original_name {
        name_mapping.insert(truncated_name.clone(), original_name);
    }
    let function = function_chunk(fields, &truncated_name, index)?;
    let param = chat_tool_param(function, tool)?;
    let mut param_object = Object::new();
    param_object.insert("type".to_owned(), Json::Text(param.kind.to_owned()));
    param_object.insert("function".to_owned(), Json::Object(param.function));
    if let Some(flag) = param.eager_input_streaming {
        param_object.insert("eager_input_streaming".to_owned(), Json::Bool(flag));
    }
    copy_cache_control(fields, &mut param_object, model);
    Ok(Json::Object(param_object))
}

fn declared_name(fields: &Object, index: usize) -> Result<String, ToolsError> {
    match fields.get("name") {
        Some(Json::Text(name)) if !name.trim().is_empty() => Ok(name.clone()),
        Some(Json::Number(number)) => Ok(number.to_string()),
        None | Some(Json::Null | Json::Text(_)) => Ok(format!("litellm_unnamed_tool_{index}")),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(ToolsError::NameNotText { index })
        }
    }
}

fn function_chunk(
    fields: &Object,
    truncated_name: &str,
    index: usize,
) -> Result<Object, ToolsError> {
    let mut function = Object::new();
    function.insert("name".to_owned(), Json::Text(truncated_name.to_owned()));
    if let Some(schema) = fields.get("input_schema") {
        match schema {
            Json::Null => {}
            Json::Object(_) => {
                function.insert("parameters".to_owned(), schema.clone());
            }
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(ToolsError::SchemaNotObject { index });
            }
        }
    }
    if let Some(description) = fields.get("description") {
        function.insert("description".to_owned(), description.clone());
    }
    if let Some(strict) = fields.get("strict") {
        function.insert("strict".to_owned(), Json::Bool(strict.is_truthy()));
    }
    let extras: Object = fields
        .iter()
        .filter(|(key, _value)| !TOOL_SCHEMA_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !extras.is_empty() {
        match function.get_mut("parameters") {
            Some(Json::Object(parameters)) => parameters.extend(extras),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(ToolsError::SchemaNotObject { index });
            }
            None => {
                function.insert("parameters".to_owned(), Json::Object(extras));
            }
        }
    }
    Ok(function)
}
