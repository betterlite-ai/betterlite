mod guardrails;

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MaskingError {
    #[error("structured MCP arguments require masked JSON")]
    InvalidMcpJson,
    #[error("MCP request has no rewritable arguments field")]
    MissingMcpArguments,
}

pub fn get_masked_text(scan_result: &Value, is_response: bool) -> Option<&str> {
    let field = if is_response {
        "response_masked_data"
    } else {
        "prompt_masked_data"
    };
    scan_result
        .get(field)
        .and_then(|masked| masked.get("data"))
        .and_then(Value::as_str)
}

pub fn mask_messages(messages: &mut [Value], masked_text: &str) {
    let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return;
    };
    mask_message_content(message, masked_text);
}

pub fn mask_response(response: &mut Value, masked_text: &str) {
    let Some(choices) = response.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message") else {
            continue;
        };
        mask_message_content(message, masked_text);
        if let Some(calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut) {
            for call in calls {
                if let Some(arguments) = call.pointer_mut("/function/arguments") {
                    *arguments = Value::String(masked_text.to_owned());
                }
            }
        }
        if let Some(arguments) = message.pointer_mut("/function_call/arguments") {
            *arguments = Value::String(masked_text.to_owned());
        }
    }
}

fn mask_message_content(message: &mut Value, masked_text: &str) {
    if let Some(content) = message.get_mut("content") {
        mask_content(content, masked_text);
    }
}

fn mask_content(content: &mut Value, masked_text: &str) {
    if content.is_string() {
        *content = Value::String(masked_text.to_owned());
    } else if let Some(parts) = content.as_array_mut() {
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                *part = serde_json::json!({"type":"text", "text":masked_text});
            }
        }
    }
}

pub fn mask_mcp_arguments(
    request_data: &mut Map<String, Value>,
    original_args: &Value,
    masked_text: &str,
) -> Result<(), MaskingError> {
    let has_arguments = request_data.contains_key("arguments");
    let has_mcp_arguments = request_data.contains_key("mcp_arguments");
    if !has_arguments && !has_mcp_arguments {
        return Err(MaskingError::MissingMcpArguments);
    }
    let replacement = if original_args.is_array() || original_args.is_object() {
        serde_json::from_str(masked_text).map_err(|_| MaskingError::InvalidMcpJson)?
    } else {
        Value::String(masked_text.to_owned())
    };
    if has_arguments {
        request_data.insert("arguments".to_owned(), replacement.clone());
    }
    if has_mcp_arguments {
        request_data.insert("mcp_arguments".to_owned(), replacement);
    }
    Ok(())
}

pub fn get_mcp_server_name(
    request_data: &Value,
    registry: &Map<String, Value>,
    tool_name: &str,
) -> String {
    for field in ["mcp_server_name", "server_name"] {
        if let Some(name) = request_data.get(field).and_then(Value::as_str) {
            if !name.is_empty() {
                return name.to_owned();
            }
        }
    }
    if let Some(server_id) = request_data.get("server_id").and_then(Value::as_str) {
        if let Some(server) = registry.get(server_id) {
            for field in ["alias", "server_name", "name", "server_id"] {
                if let Some(name) = server.get(field).and_then(Value::as_str) {
                    if !name.is_empty() {
                        return name.to_owned();
                    }
                }
            }
        }
    }
    registry
        .get(tool_name)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}
