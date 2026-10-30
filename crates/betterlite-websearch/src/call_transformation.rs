use betterlite_openai::search::records::SearchResponse;
use serde_json::{Map, Value, json};

use crate::records::WebSearchToolResultErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchErrorKind {
    RateLimit,
    BadRequest,
    Other,
}

pub struct WebSearchTransformation;

impl WebSearchTransformation {
    pub fn transform_request(response: &Value, stream: bool, format: &str) -> (bool, Vec<Value>) {
        if stream {
            return (false, Vec::new());
        }
        match format {
            "openai" => detect_openai(response),
            "responses" => detect_responses(response),
            _ => detect_anthropic(response),
        }
    }

    pub fn transform_response(calls: &[Value], results: &[String], format: &str) -> (Value, Value) {
        match format {
            "openai" => transform_openai(calls, results),
            _ => transform_anthropic(calls, results, None),
        }
    }

    pub fn transform_response_thinking(
        calls: &[Value],
        results: &[String],
        format: &str,
        thinking: Option<&[Value]>,
    ) -> (Value, Value) {
        if format == "openai" {
            transform_openai(calls, results)
        } else {
            transform_anthropic(calls, results, thinking)
        }
    }

    pub fn result_block(id: &str, response: Option<&SearchResponse>) -> Value {
        let content = response.map_or_else(Vec::new, |search| {
            search
                .results
                .iter()
                .map(|result| {
                    json!({
                        "type": "web_search_result", "url": result.url, "title": result.title,
                        "page_age": result.date.as_ref().or(result.last_updated.as_ref()),
                        "encrypted_content": "", "snippet": result.snippet,
                    })
                })
                .collect()
        });
        json!({"type": "web_search_tool_result", "tool_use_id": id, "content": content})
    }

    pub fn result_error_block(id: &str, code: WebSearchToolResultErrorCode) -> Value {
        let block = json!({"type": "web_search_tool_result", "tool_use_id": id,
            "content": {"type": "web_search_tool_result_error", "error_code": code}});
        block
    }

    pub fn search_error_code(kind: SearchErrorKind) -> WebSearchToolResultErrorCode {
        match kind {
            SearchErrorKind::RateLimit => WebSearchToolResultErrorCode::TooManyRequests,
            SearchErrorKind::BadRequest => WebSearchToolResultErrorCode::InvalidToolInput,
            SearchErrorKind::Other => WebSearchToolResultErrorCode::Unavailable,
        }
    }

    pub fn format_search_response(response: &SearchResponse) -> String {
        if response.results.is_empty() {
            return match serde_json::to_string(response) {
                Ok(serialized) => serialized,
                Err(_) => String::new(),
            };
        }
        response
            .results
            .iter()
            .map(|result| {
                format!(
                    "Title: {}\nURL: {}\nSnippet: {}",
                    result.title, result.url, result.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn detect_anthropic(response: &Value) -> (bool, Vec<Value>) {
    let Some(content) = response.get("content").and_then(Value::as_array) else {
        return (false, Vec::new());
    };
    let calls = content.iter().filter_map(|block| {
        let kind = block.get("type").and_then(Value::as_str);
        let name = block.get("name").and_then(Value::as_str);
        if kind == Some("tool_use") && matches!(name, Some("litellm_web_search" | "web_search")) {
            Some(json!({"id": block.get("id").cloned().unwrap_or(Value::Null), "type": "tool_use", "name": name,
                "input": block.get("input").cloned().unwrap_or_else(|| json!({}))}))
        } else { None }
    }).collect::<Vec<_>>();
    (!calls.is_empty(), calls)
}

fn detect_openai(response: &Value) -> (bool, Vec<Value>) {
    let Some(calls) = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
    else {
        return (false, Vec::new());
    };
    let found = calls.iter().filter_map(|call| {
        let function = call.get("function")?;
        let name = function.get("name").and_then(Value::as_str)?;
        if call.get("type").and_then(Value::as_str) != Some("function") || !matches!(name, "litellm_web_search" | "web_search") { return None; }
        let input = normalize_arguments(function.get("arguments"));
        Some(json!({"id": call.get("id").cloned().unwrap_or(Value::Null), "type": "function", "name": name,
            "function": {"name": name, "arguments": input.clone()}, "input": input}))
    }).collect::<Vec<_>>();
    (!found.is_empty(), found)
}

fn detect_responses(response: &Value) -> (bool, Vec<Value>) {
    let Some(items) = response.get("output").and_then(Value::as_array) else {
        return (false, Vec::new());
    };
    let found = items.iter().filter_map(|item| {
        if item.get("type").and_then(Value::as_str) != Some("function_call") || item.get("name").and_then(Value::as_str) != Some("litellm_web_search") { return None; }
        let arguments = item.get("arguments");
        let input = normalize_arguments(arguments);
        let arguments = match arguments {
            Some(Value::String(text)) => text.clone(),
            _ => input.to_string(),
        };
        Some(json!({"id": item.get("call_id").cloned().unwrap_or(Value::Null), "call_id": item.get("call_id").cloned().unwrap_or(Value::Null),
            "type": "function_call", "name": "litellm_web_search", "arguments": arguments, "input": input}))
    }).collect::<Vec<_>>();
    (!found.is_empty(), found)
}

fn normalize_arguments(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    }
}

fn transform_anthropic(
    calls: &[Value],
    results: &[String],
    thinking: Option<&[Value]>,
) -> (Value, Value) {
    let mut content = thinking.map_or_else(Vec::new, ToOwned::to_owned);
    content.extend(calls.iter().map(|call| json!({"type": "tool_use", "id": call.get("id"), "name": call.get("name"), "input": call.get("input").unwrap_or(&Value::Object(Map::new()))})));
    let result_content = calls.iter().enumerate().map(|(index, call)| json!({"type": "tool_result", "tool_use_id": call.get("id"), "content": results.get(index).cloned().unwrap_or_default()})).collect::<Vec<_>>();
    (
        json!({"role": "assistant", "content": content}),
        json!({"role": "user", "content": result_content}),
    )
}

fn transform_openai(calls: &[Value], results: &[String]) -> (Value, Value) {
    let tool_calls = calls.iter().map(|call| json!({"id": call.get("id"), "type": "function", "function": {"name": call.get("name"), "arguments": call.get("input").cloned().unwrap_or_else(|| json!({})).to_string()}})).collect::<Vec<_>>();
    let messages = calls.iter().enumerate().map(|(index, call)| json!({"role": "tool", "tool_call_id": call.get("id"), "content": results.get(index).cloned().unwrap_or_default()})).collect::<Vec<_>>();
    (
        json!({"role": "assistant", "tool_calls": tool_calls}),
        Value::Array(messages),
    )
}
