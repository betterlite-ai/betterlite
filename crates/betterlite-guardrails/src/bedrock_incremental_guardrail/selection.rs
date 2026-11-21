use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};

use crate::bedrock_apply::ApiInvocation;
use crate::bedrock_message_selection::wire::{Choice, Content, Message, ModelResponse, Role};
use crate::bedrock_message_selection::{Scan, SelectionContext, select};
use crate::bedrock_records::{Extensions, MessageSelection};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

const INDEX: &str = "index";
const FINISH_REASON_KEY: &str = "finish_reason";
const FINISH_REASON: &str = "stop";

// LiteLLM's selection context: structured messages line up with texts; request_data is the fallback.
#[must_use]
pub(super) fn select_messages(
    invocation: &ApiInvocation<'_>,
    inputs: &GuardrailApiInputs,
    input_type: GuardrailInputType,
    texts: &[String],
    tool_results_only: bool,
) -> MessageSelection<Message> {
    let structured = inputs.structured_messages.as_deref();
    let context = SelectionContext {
        latest_only: invocation.guardrail.latest_role_message_only,
        scan: scan_of(input_type),
        structured_messages: structured.and_then(wire_messages),
        messages: request_messages(invocation.request_data),
        tool_results_only,
    };
    select(texts, &context)
}

#[must_use]
pub(super) fn request_messages(data: &BTreeMap<String, GuardrailValue>) -> Option<Vec<Message>> {
    match data.get("messages")? {
        GuardrailValue::Array(items) => wire_messages(items),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Object(_) => None,
    }
}

// the synthetic OUTPUT response: one assistant choice per text, carrying index and finish_reason
pub(super) fn synthetic_response(messages: &[Message]) -> Result<ModelResponse, GuardrailFailure> {
    let choices = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let mut extra = Extensions::new();
            extra.insert(INDEX.to_owned(), encode(&index)?);
            extra.insert(FINISH_REASON_KEY.to_owned(), encode(FINISH_REASON)?);
            Ok(Choice {
                message: Some(Message {
                    role: Some(Role::Assistant),
                    content: Some(Content::Text(message_text(message))),
                    extra: Extensions::new(),
                }),
                delta: None,
                text: None,
                extra,
            })
        })
        .collect::<Result<Vec<_>, GuardrailFailure>>()?;
    Ok(ModelResponse {
        choices: Some(choices),
        extra: Extensions::new(),
    })
}

fn wire_messages<T>(value: &T) -> Option<Vec<Message>>
where
    T: Serialize + ?Sized,
{
    let encoded = serde_json::to_string(value).ok()?;
    let messages = serde_json::from_str(&encoded).ok()?;
    Some(messages)
}

fn message_text(message: &Message) -> String {
    match message.content.as_ref() {
        Some(Content::Text(text)) => text.clone(),
        Some(Content::List(_)) | None => String::new(),
    }
}

fn encode<T>(value: &T) -> Result<Box<RawValue>, GuardrailFailure>
where
    T: Serialize + ?Sized,
{
    to_raw_value(value).map_err(|error| GuardrailFailure::Http {
        status: 500,
        detail: format!("the synthetic Bedrock response cannot be encoded: {error}"),
    })
}

const fn scan_of(input_type: GuardrailInputType) -> Scan {
    match input_type {
        GuardrailInputType::Request => Scan::Request,
        GuardrailInputType::Response => Scan::Response,
    }
}
