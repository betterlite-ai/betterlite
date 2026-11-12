use betterlite_prompts::arguments::json::Json;

use super::error::GeminiToolsError;
use super::records::{CallingMode, FunctionCallingConfig, Tool, ToolConfig};

pub fn map_tool_choice(value: &Json) -> Result<ToolConfig, GeminiToolsError> {
    match value {
        Json::Text(text) if text == "none" => Ok(ToolConfig::from(FunctionCallingConfig {
            mode: CallingMode::None,
            allowed_function_names: None,
        })),
        Json::Text(text) if text == "required" => Ok(ToolConfig::from(FunctionCallingConfig {
            mode: CallingMode::Any,
            allowed_function_names: None,
        })),
        Json::Text(text) if text == "auto" => Ok(ToolConfig::from(FunctionCallingConfig {
            mode: CallingMode::Auto,
            allowed_function_names: None,
        })),
        Json::Object(choice) => {
            let name = match choice.get("function") {
                None | Some(Json::Null) => String::new(),
                Some(Json::Object(function)) => match function.get("name") {
                    None | Some(Json::Null) => String::new(),
                    Some(Json::Text(name)) => name.clone(),
                    Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                        return Err(GeminiToolsError::ToolChoiceName);
                    }
                },
                Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
                    return Err(GeminiToolsError::ToolChoiceFunction);
                }
            };
            Ok(ToolConfig::from(FunctionCallingConfig {
                mode: CallingMode::Any,
                allowed_function_names: Some(vec![name]),
            }))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(GeminiToolsError::UnsupportedToolChoice)
        }
    }
}

#[must_use]
pub fn map_service_tier(value: &str) -> String {
    match value.to_lowercase().as_str() {
        "auto" => "priority".to_owned(),
        other => other.to_owned(),
    }
}

pub fn drop_mixed_search_tools(tools: &mut Vec<Tool>, include_server_side: bool) -> bool {
    if include_server_side || tools.is_empty() {
        return false;
    }
    let has_functions = tools.iter().any(|tool| {
        matches!(tool, Tool::FunctionDeclarations { function_declarations } if !function_declarations.is_empty())
    });
    if !has_functions {
        return false;
    }
    if !tools.iter().any(is_search_tool) {
        return false;
    }
    tracing::warn!(
        "function declarations cannot be mixed with search tools; dropping googleSearch, googleSearchRetrieval, enterpriseWebSearch and url_context"
    );
    tools.retain(|tool| !is_search_tool(tool));
    true
}

const fn is_search_tool(tool: &Tool) -> bool {
    matches!(
        tool,
        Tool::GoogleSearch { .. }
            | Tool::GoogleSearchRetrieval { .. }
            | Tool::EnterpriseWebSearch { .. }
            | Tool::UrlContext { .. }
    )
}
