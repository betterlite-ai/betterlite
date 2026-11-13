use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::CatoError;
use super::inspection::{ExtraField, extra_inspection_sources};
use super::schema_strings::apply_schema_string_redaction;
use super::wire::{AnalyzeResponse, RedactedMessage};
use crate::content_inspection::{apply_redacted_messages_back, is_string_batch_input};

// LiteLLM's _anonymize_request
pub(super) fn anonymize_request(
    response: &AnalyzeResponse,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<(), CatoError> {
    let Some(chat) = response.redacted_chat.as_ref() else {
        return Ok(());
    };
    if chat.is_empty() {
        return Ok(());
    }
    let redacted = match chat.get("all_redacted_messages") {
        Some(GuardrailValue::Array(items)) => items
            .iter()
            .map(RedactedMessage::from_value)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let sources = extra_inspection_sources(data);
    let inspected = sources
        .iter()
        .map(|source| source.messages.len())
        .sum::<usize>();
    if is_string_batch_input(data) && redacted.len() != inspected {
        return Err(CatoError::RedactionMismatch);
    }
    let mut offset = 0;
    if let Some(GuardrailValue::Array(messages)) = data.get("messages")
        && !messages.is_empty()
    {
        let rewritten = messages
            .iter()
            .enumerate()
            .map(|(index, original)| {
                let content = redacted
                    .get(index)
                    .and_then(|message| message.content.as_ref());
                match (original, content) {
                    (GuardrailValue::Object(object), Some(content)) => {
                        let mut copy = object.clone();
                        copy.insert("content".to_owned(), content.clone());
                        GuardrailValue::Object(copy)
                    }
                    _ => original.clone(),
                }
            })
            .collect::<Vec<_>>();
        offset = messages.len();
        data.insert("messages".to_owned(), GuardrailValue::Array(rewritten));
    }
    for source in sources {
        let end = offset.saturating_add(source.messages.len());
        let slice = redacted.get(offset..end).unwrap_or_default();
        offset = end;
        match source.field {
            ExtraField::Input => {
                let rewritten = data
                    .get("input")
                    .cloned()
                    .and_then(|input| redacted_input(&input, slice));
                let Some(rewritten) = rewritten else {
                    return Err(CatoError::RedactionMismatch);
                };
                data.insert("input".to_owned(), rewritten);
            }
            ExtraField::Instructions => {
                if let Some(content) = slice.first().and_then(|message| message.content.as_ref()) {
                    data.insert("instructions".to_owned(), content.clone());
                }
            }
            ExtraField::Prompt => apply_prompt_redaction(data, slice)?,
            ExtraField::SchemaStrings => apply_schema_string_redaction(data, slice),
        }
    }
    Ok(())
}

fn redacted_input(input: &GuardrailValue, redacted: &[RedactedMessage]) -> Option<GuardrailValue> {
    let mut input_only = BTreeMap::from([("input".to_owned(), input.clone())]);
    if redacted.is_empty() {
        return (!is_string_batch_input(&input_only)).then(|| input.clone());
    }
    let values = redacted
        .iter()
        .map(RedactedMessage::to_value)
        .collect::<Vec<_>>();
    if !apply_redacted_messages_back(&mut input_only, &values) {
        return None;
    }
    input_only.remove("input")
}

fn apply_prompt_redaction(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted: &[RedactedMessage],
) -> Result<(), CatoError> {
    if redacted.is_empty() {
        return Ok(());
    }
    let contents = redacted
        .iter()
        .map(|message| message.content.as_ref())
        .collect::<Vec<_>>();
    match data.get("prompt").cloned() {
        Some(GuardrailValue::Text(_)) => {
            if let Some(Some(content)) = contents.first() {
                data.insert("prompt".to_owned(), (*content).clone());
            }
        }
        Some(GuardrailValue::Array(parts)) => {
            let rewritten = rewrite_prompt_parts(parts, &contents);
            data.insert("prompt".to_owned(), GuardrailValue::Array(rewritten));
        }
        _ => {}
    }
    Ok(())
}

fn rewrite_prompt_parts(
    parts: Vec<GuardrailValue>,
    contents: &[Option<&GuardrailValue>],
) -> Vec<GuardrailValue> {
    let mut redactions = contents.iter();
    let mut rewritten = parts;
    for part in &mut rewritten {
        if let GuardrailValue::Text(text) = part
            && !text.is_empty()
        {
            let Some(replacement) = redactions.next() else {
                break;
            };
            if let Some(content) = replacement {
                *part = content.clone();
            }
        }
    }
    rewritten
}
