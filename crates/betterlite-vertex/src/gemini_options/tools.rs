use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_tools::assembly::map_tools;
use crate::gemini_tools::options::{map_tool_choice, map_web_search_options};
use crate::gemini_tools::records::ToolConfig;

use super::convert;
use super::error::GeminiOptionsError;

// LiteLLM recognizes both wire spellings when it looks for a search tool
const SEARCH_TOOL_KEYS: [&str; 8] = [
    "googleSearch",
    "googleSearchRetrieval",
    "enterpriseWebSearch",
    "url_context",
    "google_search",
    "google_search_retrieval",
    "enterprise_web_search",
    "urlContext",
];

// LiteLLM appends each mapped list to whatever the caller already put under tools
pub fn append(
    value: &Json,
    optional: &mut Object,
    schema: &SchemaSettings,
) -> Result<usize, GeminiOptionsError> {
    let tools = match value {
        Json::Null => return Ok(0),
        Json::List(items) if items.is_empty() => return Ok(0),
        Json::List(items) => tool_objects(items)?,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(GeminiOptionsError::FieldType {
                param: "tools".to_owned(),
                expected: "a list of objects",
            });
        }
    };
    let include_server_side = matches!(
        optional.get("include_server_side_tool_invocations"),
        Some(Json::Bool(true))
    );
    let mapped = map_tools(&tools, include_server_side, tool_config(optional)?, schema)?;
    let mut encoded = Vec::with_capacity(mapped.tools.len());
    for tool in mapped.tools {
        encoded.push(convert::encode(tool)?);
    }
    append_tools(optional, encoded)?;
    if let Some(config) = mapped.tool_config {
        optional.insert("toolConfig".to_owned(), convert::encode(config)?);
    }
    Ok(mapped.warnings)
}

fn tool_objects(items: &[Json]) -> Result<Vec<Object>, GeminiOptionsError> {
    let mut tools = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(tool) = item else {
            return Err(GeminiOptionsError::FieldType {
                param: "tools".to_owned(),
                expected: "a list of objects",
            });
        };
        tools.push(tool.clone());
    }
    Ok(tools)
}

fn append_tools(optional: &mut Object, mapped: Vec<Json>) -> Result<(), GeminiOptionsError> {
    match optional.get_mut("tools") {
        Some(Json::List(existing)) => existing.extend(mapped),
        Some(_) => {
            return Err(GeminiOptionsError::NotList {
                param: "tools".to_owned(),
            });
        }
        None => {
            optional.insert("tools".to_owned(), Json::List(mapped));
        }
    }
    Ok(())
}

fn tool_config(optional: &Object) -> Result<Option<ToolConfig>, GeminiOptionsError> {
    match optional.get("toolConfig") {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(convert::decode(value, "toolConfig")?)),
    }
}

pub fn map_choice(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Null => Ok(()),
        Json::Text(_) | Json::Object(_) => {
            let config = map_tool_choice(value)?;
            optional.insert("tool_choice".to_owned(), convert::encode(config)?);
            Ok(())
        }
        Json::Bool(_) | Json::Number(_) | Json::List(_) => Err(GeminiOptionsError::FieldType {
            param: "tool_choice".to_owned(),
            expected: "a string or an object",
        }),
    }
}

// Gemini rejects parallel_tool_calls=false with several tools, so LiteLLM drops it instead
pub fn map_parallel(
    value: &Json,
    non_default: &Object,
    optional: &mut Object,
) -> Result<(), GeminiOptionsError> {
    let Json::Bool(enabled) = value else {
        return Err(GeminiOptionsError::FieldType {
            param: "parallel_tool_calls".to_owned(),
            expected: "a boolean",
        });
    };
    let count = match non_default
        .get("tools")
        .or_else(|| non_default.get("functions"))
    {
        Some(Json::List(tools)) => tools.len(),
        _ => 0,
    };
    if *enabled || count <= 1 {
        optional.insert("parallel_tool_calls".to_owned(), Json::Bool(*enabled));
    }
    Ok(())
}

pub fn map_web_search(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Object(options) => {
            let tool = map_web_search_options(options);
            append_tools(optional, vec![convert::encode(tool)?])?;
            Ok(())
        }
        Json::Null => Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(GeminiOptionsError::FieldType {
                param: "web_search_options".to_owned(),
                expected: "an object",
            })
        }
    }
}

// The last pass over the merged list, because tools and web_search_options may be mapped apart
pub fn drop_mixed_search(optional: &mut Object) -> bool {
    if matches!(
        optional.get("include_server_side_tool_invocations"),
        Some(Json::Bool(true))
    ) {
        return false;
    }
    let Some(Json::List(tools)) = optional.get_mut("tools") else {
        return false;
    };
    if !tools.iter().any(declares_functions) || !tools.iter().any(is_search) {
        return false;
    }
    tracing::warn!(
        "function declarations cannot be mixed with search tools; dropping googleSearch, googleSearchRetrieval, enterpriseWebSearch and urlContext"
    );
    tools.retain(|tool| !is_search(tool));
    true
}

fn declares_functions(tool: &Json) -> bool {
    tool.as_object()
        .and_then(|fields| fields.get("function_declarations"))
        .is_some_and(Json::is_truthy)
}

fn is_search(tool: &Json) -> bool {
    tool.as_object()
        .is_some_and(|fields| SEARCH_TOOL_KEYS.iter().any(|key| fields.contains_key(*key)))
}
