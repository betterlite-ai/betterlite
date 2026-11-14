use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImagePolicyError;
use crate::gemini_options::tools::drop_mixed_search;
use crate::gemini_tools::assembly::map_tools;
use crate::gemini_tools::options::map_web_search_options;

// LiteLLM's search-tool set, in the order its source lists the keys
const SEARCH_KEYS: [&str; 8] = [
    "googleSearch",
    "googleSearchRetrieval",
    "enterpriseWebSearch",
    "url_context",
    "google_search",
    "google_search_retrieval",
    "enterprise_web_search",
    "urlContext",
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
    drop_mixed_search(&mut result);
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
    let encoded = serde_json::to_value(&mapped.tools)
        .and_then(serde_json::from_value::<Vec<Json>>)
        .map_err(GeminiImagePolicyError::Encode)?;
    add_tools(result, encoded)?;
    if let Some(tool_config) = &mapped.tool_config {
        let encoded = serde_json::to_value(tool_config)
            .and_then(serde_json::from_value::<Json>)
            .map_err(GeminiImagePolicyError::Encode)?;
        result.insert("toolConfig".to_owned(), encoded);
    }
    Ok(())
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
        && tools.iter().any(|tool| search_key(tool).is_some())
    {
        return Ok(());
    }
    let encoded = serde_json::to_value(map_web_search_options(options))
        .and_then(serde_json::from_value::<Json>)
        .map_err(GeminiImagePolicyError::Encode)?;
    add_tools(result, vec![encoded])
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
