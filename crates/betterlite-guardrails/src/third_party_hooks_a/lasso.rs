use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::error::ThirdPartyHookError;
use crate::content_inspection::{build_inspection_messages, has_non_string_content};
use crate::lasso::wire::{LassoRequestData, MessageType};
use crate::lasso::{ConversationCache, LassoGuardrail};
use crate::lasso_masking::lifecycle::{self, Endpoint};
use crate::lasso_masking::wire::{LassoContent, LassoMessage, Message, ModelResponseShape};

mod errors;

use errors::api_error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LassoRequestHook {
    PreCall,
    Moderation,
}

pub enum PostCallResponse<'response> {
    ChatCompletion(&'response mut ModelResponseShape),
    Unexpected,
}

#[derive(Clone, Copy)]
struct Inspection<'plan> {
    request_data: &'plan LassoRequestData,
    message_type: MessageType,
    messages: &'plan [LassoMessage],
    endpoint: Endpoint,
}

pub async fn hook<C>(
    guardrail: &LassoGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    cache: &C,
    request_hook: LassoRequestHook,
    should_run: bool,
) -> Result<(), ThirdPartyHookError>
where
    C: ConversationCache,
{
    if !should_run {
        return Ok(());
    }
    if request_hook == LassoRequestHook::PreCall {
        let call_id = litellm_call_id(data)?;
        // LiteLLM warms the cache here so the header and the payload share one conversation id
        drop(guardrail.conversation_id(call_id.as_deref(), cache));
    }
    run(guardrail, data, cache, MessageType::Prompt).await
}

pub async fn post_call_success<C>(
    guardrail: &LassoGuardrail,
    data: &BTreeMap<String, GuardrailValue>,
    response: PostCallResponse<'_>,
    cache: &C,
    should_run: bool,
) -> Result<(), ThirdPartyHookError>
where
    C: ConversationCache,
{
    if !should_run {
        return Ok(());
    }
    let PostCallResponse::ChatCompletion(response) = response else {
        tracing::warn!("Unexpected response type for post-call hook");
        return Ok(());
    };
    let plan = lifecycle::post_call(response, guardrail.settings.mask, None);
    if plan.inspected.is_empty() {
        tracing::warn!("No response messages found to validate");
        return Ok(());
    }
    let request_data = LassoRequestData {
        litellm_call_id: litellm_call_id(data)?,
        tools: Vec::new(),
    };
    let masked = inspect(
        guardrail,
        cache,
        Inspection {
            request_data: &request_data,
            message_type: plan.message_type,
            messages: &plan.inspected,
            endpoint: plan.endpoint,
        },
    )
    .await;
    if guardrail.settings.mask {
        return apply_post_call_masking(response, masked);
    }
    masked?;
    Ok(())
}

fn apply_post_call_masking(
    response: &mut ModelResponseShape,
    masked: Result<Option<Vec<LassoMessage>>, ThirdPartyHookError>,
) -> Result<(), ThirdPartyHookError> {
    match masked {
        Ok(Some(masked)) => {
            *response = lifecycle::post_call(response, true, Some(&masked)).response;
            tracing::debug!("Applied Lasso masking to model response");
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(error @ ThirdPartyHookError::Rejection { .. }) => Err(error),
        Err(error) => Err(ThirdPartyHookError::Failure {
            message: format!("Failed to apply post-call masking: {error}"),
        }),
    }
}

async fn run<C>(
    guardrail: &LassoGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    cache: &C,
    message_type: MessageType,
) -> Result<(), ThirdPartyHookError>
where
    C: ConversationCache,
{
    let messages: Vec<Message> = match data.get("messages") {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(messages) => round_trip("messages", messages)?,
    };
    let input = text_input(data).map(str::to_owned);
    let mask = guardrail.settings.mask && !has_non_string_content(data);
    let mut plan = lifecycle::run(&messages, input.as_deref(), mask, None);
    append_input_inspection(data, &mut plan.inspected);
    if plan.inspected.is_empty() {
        return Ok(());
    }
    let request_data = LassoRequestData {
        litellm_call_id: litellm_call_id(data)?,
        tools: match data.get("tools") {
            None | Some(GuardrailValue::Null) => Vec::new(),
            Some(tools) => round_trip("tools", tools)?,
        },
    };
    let masked = inspect(
        guardrail,
        cache,
        Inspection {
            request_data: &request_data,
            message_type,
            messages: &plan.inspected,
            endpoint: plan.endpoint,
        },
    )
    .await?;
    if let Some(masked) = masked {
        apply_masking(data, &messages, input.as_deref(), &masked)?;
    }
    Ok(())
}

fn apply_masking(
    data: &mut BTreeMap<String, GuardrailValue>,
    messages: &[Message],
    input: Option<&str>,
    masked: &[LassoMessage],
) -> Result<(), ThirdPartyHookError> {
    let updated = lifecycle::run(messages, input, true, Some(masked));
    let write_back = matches!(
        data.get("messages"),
        Some(GuardrailValue::Array(original)) if !original.is_empty()
    );
    if write_back {
        let written: GuardrailValue = round_trip("messages", &updated.messages)?;
        data.insert("messages".to_owned(), written);
    }
    if let Some(updated_input) = updated.input {
        data.insert("input".to_owned(), GuardrailValue::Text(updated_input));
    }
    tracing::debug!(masked_fields = masked.len(), "Lasso masking applied");
    Ok(())
}

async fn inspect<C>(
    guardrail: &LassoGuardrail,
    cache: &C,
    inspection: Inspection<'_>,
) -> Result<Option<Vec<LassoMessage>>, ThirdPartyHookError>
where
    C: ConversationCache,
{
    let headers = guardrail
        .prepare_headers(inspection.request_data.litellm_call_id.as_deref(), cache)
        .map_err(api_error)?;
    let payload = guardrail.prepare_payload(
        raw_messages(inspection.messages)?,
        inspection.request_data,
        inspection.message_type,
        cache,
    );
    if inspection.endpoint == Endpoint::Classify {
        let response = guardrail
            .call_api(headers, &payload, None)
            .await
            .map_err(api_error)?;
        guardrail.process_response(response).map_err(api_error)?;
        return Ok(None);
    }
    let answer = guardrail
        .call_classifix(headers, &payload)
        .await
        .map_err(api_error)?;
    let violations = answer.response.violations_detected;
    guardrail
        .process_response(answer.response)
        .map_err(api_error)?;
    Ok(answer
        .messages
        .filter(|messages| violations && !messages.is_empty()))
}

fn append_input_inspection(
    data: &BTreeMap<String, GuardrailValue>,
    inspected: &mut Vec<LassoMessage>,
) {
    let Some(input) = data.get("input") else {
        return;
    };
    if matches!(input, GuardrailValue::Null | GuardrailValue::Text(_)) {
        return;
    }
    let input_only = BTreeMap::from([("input".to_owned(), input.clone())]);
    for message in build_inspection_messages(&input_only) {
        inspected.push(LassoMessage {
            role: message.role,
            content: LassoContent::Text(message.content),
        });
    }
}

fn text_input(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match data.get("input") {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

fn litellm_call_id(
    data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<String>, ThirdPartyHookError> {
    match data.get("litellm_call_id") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(id)) => Ok(Some(id.clone())),
        Some(_) => Err(ThirdPartyHookError::Failure {
            message: "the Lasso request field litellm_call_id has the wrong type".to_owned(),
        }),
    }
}

fn round_trip<T, R>(field: &str, value: &T) -> Result<R, ThirdPartyHookError>
where
    T: Serialize + ?Sized,
    R: DeserializeOwned,
{
    let encoded = serde_json::to_string(value).map_err(|error| ThirdPartyHookError::Failure {
        message: format!("the Lasso request field {field} could not be encoded: {error}"),
    })?;
    serde_json::from_str(&encoded).map_err(|error| ThirdPartyHookError::Failure {
        message: format!("the Lasso request field {field} has the wrong type: {error}"),
    })
}

fn raw_messages(messages: &[LassoMessage]) -> Result<Vec<Box<RawValue>>, ThirdPartyHookError> {
    messages
        .iter()
        .map(serde_json::value::to_raw_value)
        .collect::<Result<_, _>>()
        .map_err(|error| ThirdPartyHookError::Failure {
            message: format!("the Lasso request messages could not be encoded: {error}"),
        })
}
