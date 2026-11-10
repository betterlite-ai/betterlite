use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::error::GeminiToolInvokesError;
use betterlite_prompts::gemini_tool_invokes::invoke::convert_tool_calls_to_gemini;
use betterlite_prompts::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use betterlite_prompts::gemini_tool_invokes::signatures::thought_signature_from_tool;
use betterlite_prompts::gemini_tool_invokes::types::GeminiTool;
use betterlite_prompts::gemini_tool_results::types::LastToolCallMessage;
use betterlite_prompts::message_content::Message;

use crate::gemini_history::content::{bounded, object, object_of};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::turns::AssistantOptions;
use crate::gemini_media::equivalence::part_exists;

const TOOL_CALLS: &str = "tool_calls";
const FUNCTION_CALL: &str = "function_call";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const SERVER_INVOCATIONS: &str = "server_side_tool_invocations";
const TOOL_TYPE: &str = "tool_type";
const ID: &str = "id";
const ARGS: &str = "args";
const RESPONSE: &str = "response";
const THOUGHT_SIGNATURE: &str = "thought_signature";
const RESPONSE_THOUGHT_SIGNATURE: &str = "response_thought_signature";
const EXCLUDED: [&str; 1] = ["thoughtSignature"];

fn invocations_of(fields: &Object) -> Result<&[Json], HistoryError> {
    if let Some(Json::Object(provider)) = fields.get(PROVIDER_FIELDS)
        && let Some(Json::List(invocations)) = provider.get(SERVER_INVOCATIONS)
    {
        return bounded(invocations);
    }
    Ok(&[])
}

pub fn append_tool_calls(
    message: &Message,
    fields: &Object,
    options: AssistantOptions<'_>,
    parts: &mut Vec<Object>,
    last_tool_calls: &mut Option<LastToolCallMessage>,
) -> Result<(), HistoryError> {
    for part in
        convert_tool_calls_to_gemini(message, options.model, options.forward_function_call_id)?
    {
        let candidate = object_of(part)?;
        if !part_exists(parts.as_slice(), &candidate, &EXCLUDED) {
            parts.push(candidate);
        }
    }
    let carries = matches!(fields.get(TOOL_CALLS), Some(Json::List(tools)) if !tools.is_empty())
        || fields
            .get(FUNCTION_CALL)
            .is_some_and(|value| !matches!(value, Json::Null));
    if carries {
        let tool_calls = match fields.get(TOOL_CALLS) {
            Some(Json::List(tools)) => tools
                .iter()
                .filter_map(|tool| {
                    let encoded = serde_json::to_value(tool).ok()?;
                    serde_json::from_value(encoded).ok()
                })
                .collect(),
            _ => Vec::new(),
        };
        *last_tool_calls = Some(LastToolCallMessage {
            tool_calls: Some(tool_calls),
        });
    }
    Ok(())
}

pub fn collect_tool_signatures(fields: &Object) -> Result<BTreeSet<String>, HistoryError> {
    let mut signatures = BTreeSet::new();
    if let Some(Json::List(tools)) = fields.get(TOOL_CALLS) {
        if tools.len() > MAX_TOOL_CALLS {
            return Err(HistoryError::ToolInvoke(
                GeminiToolInvokesError::TooManyToolCalls {
                    limit: MAX_TOOL_CALLS,
                },
            ));
        }
        for tool in tools.iter().filter_map(|tool| {
            let encoded = serde_json::to_value(tool).ok()?;
            serde_json::from_value(encoded).ok()
        }) {
            if let Some(signature) = thought_signature_from_tool(&tool) {
                signatures.insert(signature);
            }
        }
    }
    if let Some(function_call) = fields.get(FUNCTION_CALL) {
        let wrapped = Json::Object(object!("function" => function_call.clone()));
        let encoded = serde_json::to_value(&wrapped).ok();
        if let Some(signature) = encoded
            .and_then(|raw| serde_json::from_value::<GeminiTool>(raw).ok())
            .and_then(|tool| thought_signature_from_tool(&tool))
        {
            signatures.insert(signature);
        }
    }
    add_invocation_signatures(fields, &mut signatures)?;
    Ok(signatures)
}

fn add_invocation_signatures(
    fields: &Object,
    signatures: &mut BTreeSet<String>,
) -> Result<(), HistoryError> {
    for invocation in invocations_of(fields)? {
        let Some(entry) = invocation.as_object() else {
            continue;
        };
        for name in [THOUGHT_SIGNATURE, RESPONSE_THOUGHT_SIGNATURE] {
            if let Some(Json::Text(signature)) = entry.get(name)
                && !signature.is_empty()
            {
                signatures.insert(signature.clone());
            }
        }
    }
    Ok(())
}

pub fn append_invocations(fields: &Object, parts: &mut Vec<Object>) -> Result<(), HistoryError> {
    for invocation in invocations_of(fields)? {
        let Some(entry) = invocation.as_object() else {
            continue;
        };
        parts.push(call_part(entry));
        if let Some(part) = response_part(entry) {
            parts.push(part);
        }
    }
    Ok(())
}

fn call_part(invocation: &Object) -> Object {
    let call = object!(
        "toolType" => invocation.get(TOOL_TYPE).cloned().unwrap_or(Json::Null),
        ID => invocation.get(ID).cloned().unwrap_or(Json::Null),
        ARGS => invocation.get(ARGS).cloned().unwrap_or(Json::Null),
    );
    signed_part("toolCall", call, invocation.get(THOUGHT_SIGNATURE))
}

fn response_part(invocation: &Object) -> Option<Object> {
    let response = invocation.get(RESPONSE)?;
    let mut fields = object!(
        ID => invocation.get(ID).cloned().unwrap_or(Json::Null),
        RESPONSE => response.clone(),
    );
    if let Some(tool_type) = invocation
        .get(TOOL_TYPE)
        .filter(|value| !matches!(value, Json::Null))
    {
        fields.insert("toolType".to_owned(), tool_type.clone());
    }
    Some(signed_part(
        "toolResponse",
        fields,
        invocation.get(RESPONSE_THOUGHT_SIGNATURE),
    ))
}

fn signed_part(key: &str, body: Object, carried: Option<&Json>) -> Object {
    let mut part = object!(key => Json::Object(body));
    if let Some(signature) = carried {
        part.insert("thoughtSignature".to_owned(), signature.clone());
    }
    part
}
