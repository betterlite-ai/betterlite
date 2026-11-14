use betterlite_anthropic::response_content::project_response;
use betterlite_anthropic::response_content::wire::{
    AnthropicCompletionResponse, AnthropicToolCall,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::CopilotChatError;

// a reply as Copilot sent it: only choices is read, every other field passes through untouched
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NativeReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Json>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Json>,
}

impl NativeReply {
    // LiteLLM leaves a reply alone when choices is truthy, so absent, null and an empty array are repaired
    #[must_use]
    pub fn has_choices(&self) -> bool {
        match &self.choices {
            None => false,
            Some(Json::List(choices)) => !choices.is_empty(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            ) => true,
        }
    }
}

// Newer Copilot Claude models answer with content blocks and a stop_reason instead of choices
pub fn transform_parsed_response(mut reply: NativeReply) -> Result<NativeReply, CopilotChatError> {
    if reply.has_choices() {
        return Ok(reply);
    }
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    let mut thinking_blocks = None;
    if let Some(Json::List(blocks)) = reply.fields.get("content") {
        let response = AnthropicCompletionResponse {
            content: blocks.clone(),
            fields: IndexMap::new(),
        };
        let projected = project_response(&response, None)?;
        content = projected.content;
        tool_calls = projected.tool_calls;
        thinking_blocks = projected.thinking_blocks;
    } else if let Some(Json::Text(text)) = reply.fields.get("content") {
        content.clone_from(text);
    }
    let mapped = if let Some(Json::Text(reason)) = reply.fields.get("stop_reason") {
        finish_reason_for(reason)
    } else {
        None
    };
    let finish_reason = if !tool_calls.is_empty() {
        "tool_calls"
    } else if let Some(known) = mapped {
        known
    } else if content.is_empty() {
        "length"
    } else {
        "stop"
    };
    let mut message = IndexMap::new();
    message.insert(String::from("role"), Json::Text(String::from("assistant")));
    // a tool-only reply has null content, an empty reply keeps an empty string
    message.insert(
        String::from("content"),
        if content.is_empty() && !tool_calls.is_empty() {
            Json::Null
        } else {
            Json::Text(content)
        },
    );
    if !tool_calls.is_empty() {
        message.insert(
            String::from("tool_calls"),
            Json::List(tool_calls.iter().map(tool_call_json).collect()),
        );
    }
    if let Some(blocks) = thinking_blocks.filter(|blocks| !blocks.is_empty()) {
        message.insert(String::from("thinking_blocks"), Json::List(blocks));
    }
    let choice = IndexMap::from([
        (String::from("index"), Json::Number(Number::from(0_u8))),
        (String::from("message"), Json::Object(message)),
        (
            String::from("finish_reason"),
            Json::Text(String::from(finish_reason)),
        ),
    ]);
    reply.choices = Some(Json::List(vec![Json::Object(choice)]));
    if let Some(Json::Object(usage)) = reply.fields.get_mut("usage") {
        normalize_usage(usage)?;
    }
    Ok(reply)
}

fn finish_reason_for(stop_reason: &str) -> Option<&'static str> {
    match stop_reason {
        "end_turn" | "stop_sequence" => Some("stop"),
        "max_tokens" => Some("length"),
        "tool_use" => Some("tool_calls"),
        _ => None,
    }
}

fn tool_call_json(call: &AnthropicToolCall) -> Json {
    let function = IndexMap::from([
        (
            String::from("name"),
            call.function.name.clone().map_or(Json::Null, Json::Text),
        ),
        (
            String::from("arguments"),
            call.function
                .arguments
                .clone()
                .map_or(Json::Null, Json::Text),
        ),
    ]);
    let mut fields = IndexMap::from([
        (
            String::from("id"),
            call.id.clone().map_or(Json::Null, Json::Text),
        ),
        (String::from("type"), Json::Text(call.r#type.clone())),
        (String::from("function"), Json::Object(function)),
        (
            String::from("index"),
            Json::Number(Number::from(call.index)),
        ),
    ]);
    if let Some(caller) = &call.caller {
        fields.insert(String::from("caller"), caller.clone());
    }
    Json::Object(fields)
}

// prompt and completion tokens come from input and output only when absent, and total only when absent
fn normalize_usage(usage: &mut IndexMap<String, Json>) -> Result<(), CopilotChatError> {
    for (native, openai) in [
        ("input_tokens", "prompt_tokens"),
        ("output_tokens", "completion_tokens"),
    ] {
        if let Some(count) = usage.get(native).cloned()
            && !usage.contains_key(openai)
        {
            usage.insert(String::from(openai), count);
        }
    }
    if !usage.contains_key("total_tokens") {
        let total = token_count(usage.get("prompt_tokens"), "prompt_tokens")?
            .checked_add(token_count(
                usage.get("completion_tokens"),
                "completion_tokens",
            )?)
            .ok_or(CopilotChatError::Usage {
                name: "total_tokens",
            })?;
        usage.insert(
            String::from("total_tokens"),
            Json::Number(Number::from(total)),
        );
    }
    Ok(())
}

fn token_count(value: Option<&Json>, name: &'static str) -> Result<i64, CopilotChatError> {
    match value {
        None => Ok(0),
        Some(Json::Number(count)) => count.as_i64().ok_or(CopilotChatError::Usage { name }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(CopilotChatError::Usage { name })
        }
    }
}
