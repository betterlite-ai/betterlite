use betterlite_models::decisions::Json;

use super::chat_tools::{
    ChatFunction, ChatFunctionTool, ChatTool, ToolExtras, WebSearchOptions, schema_parameters,
};
use super::error::NamespaceToolsError;
use super::namespace_members::namespace_chat_tools;
use super::namespace_names::check_collisions;
use super::values::{Fields, has_value, text_field, tool_objects, tool_type};
use crate::api::responses_codex_tools::custom_tools::custom_to_function_tool;

const UNSUPPORTED_BUILTINS: [&str; 5] = [
    "computer_use",
    "image_generation",
    "local_shell",
    "shell",
    "tool_search",
];
const REVERSE_EXTRAS: [&str; 5] = [
    "cache_control",
    "defer_loading",
    "allowed_callers",
    "input_examples",
    "eager_input_streaming",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatConversion {
    pub chat_tools: Vec<ChatTool>,
    pub web_search_options: Option<WebSearchOptions>,
    pub warnings: Vec<String>,
}

// LiteLLM copies the strict flag and the extras only when they hold a value (a false, 0, "", [] or {} is skipped)
fn present_value(tool: &Fields, key: &str) -> Option<Json> {
    match tool.get(key) {
        Some(value) if has_value(value) => Some(value.clone()),
        Some(_) | None => None,
    }
}

fn function_chat_tool(tool: &Fields) -> Result<ChatFunctionTool, NamespaceToolsError> {
    Ok(ChatFunctionTool {
        kind: "function",
        function: ChatFunction {
            name: text_field(tool, "name")?.to_owned(),
            description: text_field(tool, "description")?.to_owned(),
            parameters: schema_parameters(tool.get("parameters"))?,
            strict: present_value(tool, "strict").unwrap_or(Json::Bool(false)),
        },
        extras: ToolExtras {
            cache_control: present_value(tool, "cache_control"),
            defer_loading: present_value(tool, "defer_loading"),
            allowed_callers: present_value(tool, "allowed_callers"),
            input_examples: present_value(tool, "input_examples"),
            eager_input_streaming: tool
                .get("eager_input_streaming")
                .filter(|value| **value != Json::Null)
                .cloned(),
        },
    })
}

fn convert_tool(tool: &Fields, conversion: &mut ChatConversion) -> Result<(), NamespaceToolsError> {
    match tool_type(tool) {
        "web_search_preview" | "web_search" => {
            conversion.web_search_options = Some(WebSearchOptions {
                search_context_size: tool
                    .get("search_context_size")
                    .cloned()
                    .unwrap_or(Json::Null),
                user_location: present_value(tool, "user_location").unwrap_or(Json::Null),
            });
        }
        "function" => conversion
            .chat_tools
            .push(ChatTool::Function(Box::new(function_chat_tool(tool)?))),
        "namespace" => conversion.chat_tools.extend(namespace_chat_tools(tool)?),
        "custom" => conversion
            .chat_tools
            .extend(custom_to_function_tool(tool)?.map(ChatTool::Custom)),
        kind if UNSUPPORTED_BUILTINS.contains(&kind) => conversion.warnings.push(format!(
            "Dropping Responses API tool of type '{kind}': it has no Chat Completions equivalent and the target provider would reject the request."
        )),
        _ => conversion
            .chat_tools
            .push(ChatTool::Passthrough(tool.clone())),
    }
    Ok(())
}

// mcp tools and types betterlite does not know pass through as sent; the last web search tool sets the search options
pub fn responses_tools_to_chat(tools: &[Json]) -> Result<ChatConversion, NamespaceToolsError> {
    check_collisions(tools)?;
    let mut conversion = ChatConversion::default();
    for tool in tool_objects(tools)? {
        convert_tool(tool, &mut conversion)?;
    }
    Ok(conversion)
}

fn responses_function_tool(tool: &Fields) -> Result<Fields, NamespaceToolsError> {
    let function = match tool.get("function") {
        None | Some(Json::Null) => Fields::new(),
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(NamespaceToolsError::WrongType {
                field: "function",
                expected: "an object",
            });
        }
    };
    let mut responses_tool = Fields::new();
    responses_tool.insert("type".to_owned(), Json::Text("function".to_owned()));
    responses_tool.insert(
        "name".to_owned(),
        Json::Text(text_field(&function, "name")?.to_owned()),
    );
    responses_tool.insert(
        "description".to_owned(),
        Json::Text(text_field(&function, "description")?.to_owned()),
    );
    responses_tool.insert(
        "parameters".to_owned(),
        Json::Object(schema_parameters(function.get("parameters"))?),
    );
    responses_tool.insert(
        "strict".to_owned(),
        present_value(&function, "strict").unwrap_or(Json::Bool(false)),
    );
    for key in REVERSE_EXTRAS {
        if let Some(value) = tool.get(key).filter(|value| **value != Json::Null) {
            responses_tool.insert(key.to_owned(), value.clone());
        }
    }
    Ok(responses_tool)
}

// the inverse of responses_tools_to_chat for the tool list; non-function tools pass through unchanged
pub fn chat_tools_to_responses(chat_tools: &[Json]) -> Result<Vec<Fields>, NamespaceToolsError> {
    tool_objects(chat_tools)?
        .into_iter()
        .map(|tool| {
            if tool_type(tool) == "function" {
                responses_function_tool(tool)
            } else {
                Ok(tool.clone())
            }
        })
        .collect()
}
