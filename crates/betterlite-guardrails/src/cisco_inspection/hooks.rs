use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::event_hooks::{is_mcp_call_type, surface_matches};
use crate::cisco_configuration::records::Direction;
use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::request_messages;
use crate::cisco_conversation::response::{chat_messages, completion_messages, output_messages};
use crate::cisco_conversation::{Fields, InspectMessage};
use crate::cisco_inspection::chat::{ChatInspectionArgs, inspect_chat};
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::mcp::inspect_mcp_request;
use crate::cisco_verdict_enforcement::finalize::{FinalizeOutcome, ResponseSurface};
use crate::cisco_verdict_enforcement::support::insert_serialized;

pub struct HookArgs<'args> {
    pub call_type: Option<&'args str>,
    pub should_run: bool,
    pub key_user_id: Option<&'args str>,
}

// Mirrors LiteLLM's fn async_pre_call_hook.
pub async fn async_pre_call_hook(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    args: HookArgs<'_>,
) -> Option<FinalizeOutcome> {
    let is_mcp = is_mcp_call_type(args.call_type);
    if !surface_matches(guardrail.settings.inspection_type, is_mcp) || !args.should_run {
        return None;
    }
    let outcome = if is_mcp {
        inspect_mcp_request(guardrail, request_data).await?
    } else {
        let messages = request_messages(request_data);
        if messages.is_empty() {
            return None;
        }
        chat_outcome(
            guardrail,
            request_data,
            messages,
            Direction::Input,
            args.key_user_id,
        )
        .await
    };
    record_applied_guardrail(request_data, &guardrail.settings.guardrail_name);
    Some(outcome)
}

// Mirrors LiteLLM's fn async_moderation_hook, triggered for the during_call event.
pub async fn async_moderation_hook(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    args: HookArgs<'_>,
) -> Option<FinalizeOutcome> {
    let is_mcp = is_mcp_call_type(args.call_type);
    if !surface_matches(guardrail.settings.inspection_type, is_mcp) || !args.should_run {
        return None;
    }
    let outcome = if is_mcp {
        inspect_mcp_request(guardrail, request_data).await?
    } else {
        let messages = request_messages(request_data);
        if messages.is_empty() {
            return None;
        }
        chat_outcome(
            guardrail,
            request_data,
            messages,
            Direction::Input,
            args.key_user_id,
        )
        .await
    };
    record_applied_guardrail(request_data, &guardrail.settings.guardrail_name);
    Some(outcome)
}

pub struct PostCallArgs<'args> {
    pub should_run: bool,
    pub key_user_id: Option<&'args str>,
}

// Mirrors LiteLLM's fn async_post_call_success_hook.
pub async fn async_post_call_success_hook(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    response: &mut Fields,
    args: PostCallArgs<'_>,
) -> Option<FinalizeOutcome> {
    if guardrail.settings.inspection_type != InspectionType::Chat || !args.should_run {
        return None;
    }
    let response_messages = extract_response_messages(response);
    if response_messages.is_empty() {
        return None;
    }
    let mut messages = request_messages(request_data);
    messages.extend(response_messages);
    let outcome = chat_outcome_on_response(
        guardrail,
        request_data,
        response,
        messages,
        args.key_user_id,
    )
    .await;
    record_applied_guardrail(request_data, &guardrail.settings.guardrail_name);
    Some(outcome)
}

async fn chat_outcome(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    messages: Vec<InspectMessage>,
    direction: Direction,
    key_user_id: Option<&str>,
) -> FinalizeOutcome {
    let args = ChatInspectionArgs {
        messages,
        direction,
        key_user_id,
    };
    inspect_chat(guardrail, args, request_data, ResponseSurface::None).await
}

async fn chat_outcome_on_response(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    response: &mut Fields,
    messages: Vec<InspectMessage>,
    key_user_id: Option<&str>,
) -> FinalizeOutcome {
    let args = ChatInspectionArgs {
        messages,
        direction: Direction::Output,
        key_user_id,
    };
    inspect_chat(
        guardrail,
        args,
        request_data,
        ResponseSurface::ChatResponse(response),
    )
    .await
}

// Mirrors LiteLLM's fn _extract_response_messages.
fn extract_response_messages(response: &Fields) -> Vec<InspectMessage> {
    if has_choice_key(response, "message") {
        return chat_messages(response);
    }
    if has_choice_key(response, "text") {
        return completion_messages(response);
    }
    if matches!(Json::field(response, "output"), Json::List(_)) {
        return output_messages(response);
    }
    Vec::new()
}

fn has_choice_key(response: &Fields, key: &str) -> bool {
    Json::field(response, "choices")
        .objects()
        .iter()
        .any(|choice| choice.contains_key(key))
}

// Mirrors LiteLLM's fn add_guardrail_to_applied_guardrails_header.
fn record_applied_guardrail(request_data: &mut Fields, guardrail_name: &str) {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let mut bucket = match Json::field(request_data, bucket_key) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Fields::new()
        }
    };
    let mut applied = applied_guardrails(&bucket);
    if !applied.iter().any(|name| name == guardrail_name) {
        applied.push(guardrail_name.to_owned());
    }
    insert_serialized(&mut bucket, "applied_guardrails", &applied);
    insert_serialized(request_data, bucket_key, &bucket);
}

fn applied_guardrails(bucket: &Fields) -> Vec<String> {
    match Json::field(bucket, "applied_guardrails") {
        Json::List(items) => items
            .iter()
            .filter_map(|item| match Json::parse(item.get()) {
                Json::Text(name) => Some(name),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

mod tests;
