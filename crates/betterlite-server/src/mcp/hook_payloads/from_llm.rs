use betterlite_models::decisions::{HiddenParams, Json};
use regex::Regex;

use crate::mcp::hook_payloads::error::HookPayloadError;
use crate::mcp::hook_payloads::{
    GuardrailHookData, GuardrailOutcome, McpDuringCallResponse, McpPreCallResponse, McpToolCall,
    ToolArguments, tool_call_text,
};

const BLOCKED: &str = "Content blocked by guardrail";
const BLOCKED_DURING: &str = "Content blocked by guardrail during execution";
const MODIFIED_DURING: &str = "Content modified by guardrail during execution";
const ARGUMENTS_LABEL: &str = "Arguments:";

pub fn llm_result_to_pre_call(
    outcome: &GuardrailOutcome,
    call: McpToolCall<'_>,
) -> Result<Option<McpPreCallResponse>, HookPayloadError> {
    let data = match outcome {
        GuardrailOutcome::Raised(message) | GuardrailOutcome::Text(message) => {
            return Ok(Some(McpPreCallResponse {
                should_proceed: false,
                modified_arguments: None,
                error_message: Some(message.clone()),
                hidden_params: HiddenParams::new(),
            }));
        }
        GuardrailOutcome::Data(answer) => answer,
    };
    let Some(content) = changed_content(data, call)? else {
        return Ok(None);
    };
    let lowered = content.to_lowercase();
    if content.is_empty() || lowered.contains("blocked") || lowered.contains("violation") {
        return Ok(Some(McpPreCallResponse {
            should_proceed: false,
            modified_arguments: None,
            error_message: Some(BLOCKED.to_owned()),
            hidden_params: HiddenParams::new(),
        }));
    }
    let Some(arguments) = modified_arguments(content, call.arguments) else {
        tracing::warn!("Could not parse modified arguments from guardrail response: {content}");
        return Ok(None);
    };
    Ok(Some(McpPreCallResponse {
        should_proceed: true,
        modified_arguments: Some(arguments),
        error_message: None,
        hidden_params: HiddenParams::new(),
    }))
}

pub fn llm_result_to_during_call(
    outcome: &GuardrailOutcome,
    call: McpToolCall<'_>,
) -> Result<Option<McpDuringCallResponse>, HookPayloadError> {
    let error_message = match outcome {
        GuardrailOutcome::Raised(message) | GuardrailOutcome::Text(message) => message.clone(),
        GuardrailOutcome::Data(data) => {
            let Some(content) = changed_content(data, call)? else {
                return Ok(None);
            };
            // unlike the pre-call check, violation alone reads as a modification here
            if content.is_empty() || content.to_lowercase().contains("blocked") {
                BLOCKED_DURING.to_owned()
            } else {
                MODIFIED_DURING.to_owned()
            }
        }
    };
    Ok(Some(McpDuringCallResponse {
        should_continue: false,
        error_message: Some(error_message),
        hidden_params: HiddenParams::new(),
    }))
}

// the comparison text never carries the Description line, so a described tool call always reads as changed
fn changed_content<'data>(
    data: &'data GuardrailHookData,
    call: McpToolCall<'_>,
) -> Result<Option<&'data str>, HookPayloadError> {
    let Some(first) = data.messages.first() else {
        return Ok(None);
    };
    let content = first.content.as_deref().unwrap_or_default();
    if content == tool_call_text(call, None)? {
        return Ok(None);
    }
    Ok(Some(content))
}

fn modified_arguments(content: &str, original: &ToolArguments) -> Option<ToolArguments> {
    tracing::debug!("Extracting modified args from content: {content}");
    let Some(found) = content
        .trim()
        .lines()
        .find_map(|line| line.strip_prefix(ARGUMENTS_LABEL))
    else {
        tracing::warn!("Could not find 'Arguments:' line in masked content");
        return None;
    };
    let arguments_text = found.trim();
    tracing::debug!("Found arguments text: {arguments_text}");
    match serde_json::from_str::<Json>(arguments_text) {
        Ok(Json::Object(arguments)) => Some(arguments),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => None,
        Err(error) => {
            tracing::debug!("Failed to parse JSON arguments: {arguments_text}, error: {error}");
            manual_arguments(arguments_text, original)
        }
    }
}

// LiteLLM's fallback for text that is not JSON: only string arguments the text names, with a non-empty value
fn manual_arguments(arguments_text: &str, original: &ToolArguments) -> Option<ToolArguments> {
    original
        .iter()
        .map(|(key, value)| {
            let Json::Text(_) = value else {
                return Some((key.clone(), value.clone()));
            };
            let escaped = regex::escape(key);
            let matcher = Regex::new(&format!(
                r#"(?i)['"]?{escaped}['"]?\s*:\s*['"]?([^,'"]*)['"]?"#
            ))
            .inspect_err(|error| tracing::error!("Error in manual argument parsing: {error}"))
            .ok()?;
            let replaced = matcher
                .captures(arguments_text)
                .and_then(|captures| captures.get(1))
                .map(|group| group.as_str().trim())
                .filter(|text| !text.is_empty())
                .map_or_else(|| value.clone(), |text| Json::Text(text.to_owned()));
            Some((key.clone(), replaced))
        })
        .collect()
}
