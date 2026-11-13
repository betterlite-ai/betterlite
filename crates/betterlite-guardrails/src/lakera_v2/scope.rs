use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::content_inspection::{
    InspectionMessage, apply_redacted_messages_back, build_inspection_messages,
    has_non_string_content,
};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::merge_messages::merge_scoped_messages;
use crate::guardrail_message_scope::scoped_indices::{ScopeFlags, message_role};
use crate::lakera_pii::masking::{blocked_error, mask_pii_in_messages};
use crate::lakera_pii::wire::{LakeraResponse, Message};

use super::Data;
use super::error::LakeraV2Error;

pub const MULTIMODAL_REASON: &str = "multimodal content, masking would drop the image/audio parts";
pub const COMBINED_REASON: &str =
    "messages and input are both present, so the write-back is positionally ambiguous";
pub const NOT_A_LIST_REASON: &str =
    "a messages key that isn't a list, so there's nothing to merge the redacted content into";
pub const NO_WRITE_BACK_REASON: &str = "no write-back path for the redacted content";
pub const NO_PAYLOAD_REASON: &str =
    "Lakera reported no locations to redact, so payload=true is likely off";

// LiteLLM's _build_lakera_inspection_messages: instructions lead, then the shared inspection list
#[must_use]
pub fn build_lakera_inspection_messages(data: &Data) -> Vec<InspectionMessage> {
    let mut messages = Vec::new();
    if let Some(GuardrailValue::Text(instructions)) = data.get("instructions")
        && !instructions.is_empty()
    {
        messages.push(InspectionMessage {
            role: "system".to_owned(),
            content: instructions.clone(),
        });
    }
    messages.extend(build_inspection_messages(data));
    messages
}

#[must_use]
pub fn has_combined_messages_and_input(data: &Data) -> bool {
    matches!(data.get("messages"), Some(GuardrailValue::Array(_)))
        && data
            .get("input")
            .is_some_and(|input| *input != GuardrailValue::Null)
}

#[must_use]
pub fn has_responses_instructions(data: &Data, scope: ScopeFlags) -> bool {
    let Some(GuardrailValue::Text(instructions)) = data.get("instructions") else {
        return false;
    };
    !instructions.is_empty() && !scope.skip_system_message
}

#[must_use]
pub fn breakdown_has_pii_violation(response: Option<&LakeraResponse>) -> bool {
    let Some(items) = response.and_then(|value| value.breakdown.as_deref()) else {
        return false;
    };
    items.iter().any(|item| {
        item.detected == Some(true)
            && item
                .detector_type
                .as_deref()
                .is_some_and(|kind| kind.starts_with("pii/"))
    })
}

// The five obstacles in LiteLLM's order; None means the instructions-only mask can run
#[must_use]
pub fn unmaskable_reason(
    data: &Data,
    response: Option<&LakeraResponse>,
    scope: ScopeFlags,
) -> Option<&'static str> {
    if has_non_string_content(data) {
        return Some(MULTIMODAL_REASON);
    }
    if has_combined_messages_and_input(data) {
        return Some(COMBINED_REASON);
    }
    if data.contains_key("messages")
        && !matches!(data.get("messages"), Some(GuardrailValue::Array(_)))
    {
        return Some(NOT_A_LIST_REASON);
    }
    if !has_responses_instructions(data, scope) {
        return Some(NO_WRITE_BACK_REASON);
    }
    if !response
        .and_then(|value| value.payload.as_ref())
        .is_some_and(|payload| !payload.is_empty())
    {
        return Some(NO_PAYLOAD_REASON);
    }
    None
}

#[must_use]
pub fn pre_masking_scope_indices(messages: &[GuardrailValue], scope: ScopeFlags) -> Vec<usize> {
    let mut indices = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let GuardrailValue::Object(fields) = message else {
            continue;
        };
        let Some(GuardrailValue::Text(content)) = fields.get("content") else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        let role = message_role(message);
        if scope.skip_system_message && role == "system" {
            continue;
        }
        if scope.skip_tool_message && role == "tool" {
            continue;
        }
        indices.push(index);
    }
    indices
}

#[must_use]
pub fn inspection_values(messages: &[InspectionMessage]) -> Vec<GuardrailValue> {
    messages
        .iter()
        .map(|message| {
            GuardrailValue::Object(BTreeMap::from([
                (
                    "role".to_owned(),
                    GuardrailValue::Text(message.role.clone()),
                ),
                (
                    "content".to_owned(),
                    GuardrailValue::Text(message.content.clone()),
                ),
            ]))
        })
        .collect()
}

pub fn values_to_messages(values: &[GuardrailValue]) -> Result<Vec<Message>, LakeraV2Error> {
    let mut messages = Vec::with_capacity(values.len());
    for value in values {
        let text = serde_json::to_string(value).map_err(|source| LakeraV2Error::Json { source })?;
        let raw = RawValue::from_string(text).map_err(|source| LakeraV2Error::Json { source })?;
        let message = Message::try_from(raw).map_err(|source| LakeraV2Error::Json { source })?;
        messages.push(message);
    }
    Ok(messages)
}

pub fn content_text(message: &Message) -> Result<String, LakeraV2Error> {
    let Some(raw) = message.content.as_deref() else {
        return Err(LakeraV2Error::MissingContent);
    };
    serde_json::from_str(raw.get()).map_err(|source| LakeraV2Error::Json { source })
}

pub fn message_value(message: &Message) -> Result<GuardrailValue, LakeraV2Error> {
    serde_json::to_value(message)
        .and_then(serde_json::from_value::<GuardrailValue>)
        .map_err(|source| LakeraV2Error::Json { source })
}

pub fn apply_redacted_back_preserving_fields(
    data: &mut Data,
    redacted: &[Message],
    scope: ScopeFlags,
) -> Result<(), LakeraV2Error> {
    let original = match data.get("messages") {
        Some(GuardrailValue::Array(messages)) => Some(messages.clone()),
        _ => None,
    };
    let Some(original) = original else {
        let mut values = Vec::with_capacity(redacted.len());
        for message in redacted {
            values.push(message_value(message)?);
        }
        apply_redacted_messages_back(data, &values);
        return Ok(());
    };
    let indices = pre_masking_scope_indices(&original, scope);
    if indices.len() != redacted.len() {
        return Err(LakeraV2Error::ScopeMismatch {
            expected: indices.len(),
            found: redacted.len(),
        });
    }
    let mut scoped = Vec::with_capacity(indices.len());
    for (index, message) in indices.iter().zip(redacted) {
        let Some(GuardrailValue::Object(fields)) = original.get(*index) else {
            return Err(LakeraV2Error::ResponseShape { at: "messages" });
        };
        let mut fields = fields.clone();
        fields.insert(
            "content".to_owned(),
            GuardrailValue::Text(content_text(message)?),
        );
        scoped.push(GuardrailValue::Object(fields));
    }
    let merged = merge_scoped_messages(&original, &indices, &scoped);
    data.insert("messages".to_owned(), GuardrailValue::Array(merged));
    Ok(())
}

pub fn mask_unwritable_instructions_pii_in_place(
    data: &mut Data,
    inspected: &[Message],
    response: Option<&LakeraResponse>,
    counts: &mut BTreeMap<String, u64>,
    scope: ScopeFlags,
) -> Result<bool, LakeraV2Error> {
    if unmaskable_reason(data, response, scope).is_some() {
        return Ok(false);
    }
    let redacted = mask_pii_in_messages(inspected, response, counts);
    let Some(first) = redacted.first() else {
        return Err(LakeraV2Error::ResponseShape { at: "instructions" });
    };
    let instructions = content_text(first)?;
    data.insert(
        "instructions".to_owned(),
        GuardrailValue::Text(instructions),
    );
    let rest = redacted.get(1..).unwrap_or_default();
    apply_redacted_back_preserving_fields(data, rest, scope)?;
    Ok(true)
}

pub fn blocked_failure(
    response: Option<&LakeraResponse>,
) -> Result<GuardrailFailure, LakeraV2Error> {
    let error = blocked_error(response);
    let mut detail = IndexMap::new();
    detail.insert("error".to_owned(), GuardrailValue::Text(error.to_string()));
    let carried = match response {
        Some(value) => response_value(value)?,
        None => GuardrailValue::Null,
    };
    detail.insert("lakera_guardrail_response".to_owned(), carried);
    Ok(GuardrailFailure::HttpObject {
        status: error.status().as_u16(),
        detail,
    })
}

fn response_value(response: &LakeraResponse) -> Result<GuardrailValue, LakeraV2Error> {
    serde_json::to_value(response)
        .and_then(serde_json::from_value::<GuardrailValue>)
        .map_err(|source| LakeraV2Error::Json { source })
}
