use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use super::error::GeminiImagePolicyError;
use crate::gemini_tools::assembly::map_tools;
use crate::gemini_tools::options::map_web_search_options;

// LiteLLM's search-tool set, in its declaration order; url_context is absent from it
const SEARCH_KEYS: [&str; 7] = [
    "googleSearch",
    "googleSearchRetrieval",
    "enterpriseWebSearch",
    "urlContext",
    "google_search",
    "google_search_retrieval",
    "enterprise_web_search",
];

pub fn map_gemini_image_tools_params(
    non_default_params: &Object,
    mapped_params: &Object,
    settings: &Settings,
) -> Result<Object, GeminiImagePolicyError> {
    let mut result = mapped_params.clone();
    result.shift_remove("web_search_options");
    map_tools_param(non_default_params, &mut result, settings)?;
    add_web_search_tool(non_default_params, &mut result)?;
    drop_search_tools_mixed_with_functions(&mut result);
    dedupe_search_tools(&mut result);
    Ok(result)
}

fn map_tools_param(
    non_default_params: &Object,
    result: &mut Object,
    settings: &Settings,
) -> Result<(), GeminiImagePolicyError> {
    let Some(Json::List(tools)) = non_default_params
        .get("tools")
        .filter(|value| !matches!(value, Json::List(items) if items.is_empty()))
    else {
        return Ok(());
    };
    let include_server_side = matches!(
        result.get("include_server_side_tool_invocations"),
        Some(Json::Bool(true))
    );
    let objects = tools
        .iter()
        .map(|tool| match tool {
            Json::Object(object) => Ok(object.clone()),
            _ => Err(GeminiImagePolicyError::ToolsShape),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mapped = map_tools(&objects, include_server_side, None, settings)?;
    let encoded = mapped
        .tools
        .iter()
        .map(encode)
        .collect::<Result<Vec<_>, _>>()?;
    add_tools(result, encoded)?;
    if let Some(tool_config) = mapped.tool_config {
        result.insert("toolConfig".to_owned(), encode(&tool_config)?);
    }
    Ok(())
}

fn encode<T>(value: &T) -> Result<Json, GeminiImagePolicyError>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| GeminiImagePolicyError::Encode { source })?;
    serde_json::from_value(encoded).map_err(|source| GeminiImagePolicyError::Encode { source })
}

fn add_tools(result: &mut Object, tools: Vec<Json>) -> Result<(), GeminiImagePolicyError> {
    if result.contains_key("tools") {
        let Some(Json::List(existing)) = result.get_mut("tools") else {
            return Err(GeminiImagePolicyError::ToolsShape);
        };
        existing.extend(tools);
        return Ok(());
    }
    result.insert("tools".to_owned(), Json::List(tools));
    Ok(())
}

fn add_web_search_tool(
    non_default_params: &Object,
    result: &mut Object,
) -> Result<(), GeminiImagePolicyError> {
    let Some(Json::Object(options)) = non_default_params.get("web_search_options") else {
        return Ok(());
    };
    if let Some(Json::List(tools)) = result.get("tools")
        && tools.iter().any(is_search_tool)
    {
        return Ok(());
    }
    add_tools(result, vec![encode(&map_web_search_options(options))?])
}

fn is_search_tool(tool: &Json) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    SEARCH_KEYS.iter().any(|key| fields.contains_key(*key))
}

fn has_function_declarations(tool: &Json) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    matches!(
        fields.get("function_declarations"),
        Some(Json::List(declarations)) if !declarations.is_empty()
    )
}

fn drop_search_tools_mixed_with_functions(result: &mut Object) {
    let Some(Json::List(tools)) = result.get("tools") else {
        return;
    };
    let server_side = matches!(
        result.get("include_server_side_tool_invocations"),
        Some(Json::Bool(true))
    );
    if server_side
        || !tools.iter().any(has_function_declarations)
        || !tools.iter().any(is_search_tool)
    {
        return;
    }
    tracing::warn!(
        "function declarations cannot be mixed with search tools; dropping googleSearch, googleSearchRetrieval, enterpriseWebSearch and urlContext"
    );
    let kept: Vec<Json> = tools
        .iter()
        .filter(|tool| !is_search_tool(tool))
        .cloned()
        .collect();
    result.insert("tools".to_owned(), Json::List(kept));
}

fn dedupe_search_tools(result: &mut Object) {
    let Some(Json::List(tools)) = result.get("tools") else {
        return;
    };
    let mut seen: Vec<&'static str> = Vec::new();
    let mut deduped = Vec::with_capacity(tools.len());
    for tool in tools {
        let Some(key) = search_key(tool) else {
            deduped.push(tool.clone());
            continue;
        };
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        deduped.push(tool.clone());
    }
    result.insert("tools".to_owned(), Json::List(deduped));
}

fn search_key(tool: &Json) -> Option<&'static str> {
    let Json::Object(fields) = tool else {
        return None;
    };
    SEARCH_KEYS
        .iter()
        .copied()
        .find(|key| fields.contains_key(*key))
}
