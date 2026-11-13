use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiToolsError;
use super::records::{CallingMode, FunctionCallingConfig, Tool, ToolConfig};

pub fn map_tool_choice(value: &Json) -> Result<ToolConfig, GeminiToolsError> {
    match value {
        Json::Text(text) => match text.as_str() {
            "none" => Ok(ToolConfig::from(FunctionCallingConfig {
                mode: CallingMode::None,
                allowed_function_names: None,
            })),
            "required" => Ok(ToolConfig::from(FunctionCallingConfig {
                mode: CallingMode::Any,
                allowed_function_names: None,
            })),
            "auto" => Ok(ToolConfig::from(FunctionCallingConfig {
                mode: CallingMode::Auto,
                allowed_function_names: None,
            })),
            _ => Err(GeminiToolsError::UnsupportedToolChoice),
        },
        Json::Object(choice) => {
            let name = match choice.get("function") {
                None | Some(Json::Null) => String::new(),
                Some(Json::Object(function)) => match function.get("name") {
                    None | Some(Json::Null) => String::new(),
                    Some(Json::Text(name)) => name.clone(),
                    Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                        return Err(GeminiToolsError::ToolChoiceField { field: "name" });
                    }
                },
                Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
                    return Err(GeminiToolsError::ToolChoiceField { field: "function" });
                }
            };
            Ok(ToolConfig::from(FunctionCallingConfig {
                mode: CallingMode::Any,
                allowed_function_names: Some(vec![name]),
            }))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
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
#[must_use]
pub fn map_web_search_options(_options: &Object) -> Tool {
    // Gemini takes neither user_location nor search_context_size, so every options object becomes an empty googleSearch
    Tool::GoogleSearch {
        google_search: Object::new(),
    }
}
pub fn drop_mixed_search_tools(tools: &mut Vec<Tool>, include_server_side: bool) -> bool {
    if include_server_side || tools.is_empty() {
        return false;
    }
    let has_functions = tools.iter().any(|tool| {
        matches!(tool, Tool::FunctionDeclarations { function_declarations } if !function_declarations.is_empty())
    });
    if !has_functions || !tools.iter().any(is_search_tool) {
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
