use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::CatoError;
use super::inspection::{ExtraField, ExtraSource, extra_inspection_sources};
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
    let redacted = chat.all_redacted_messages.as_deref().unwrap_or_default();
    let sources = extra_inspection_sources(data);
    let inspected = sources
        .iter()
        .map(|source| source.messages.len())
        .sum::<usize>();
    if is_string_batch_input(data) && redacted.len() != inspected {
        return Err(CatoError::RedactionMismatch);
    }
    let offset = rewrite_messages(data, redacted);
    write_extra_sources(data, &sources, redacted, offset)
}

fn rewrite_messages(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted: &[RedactedMessage],
) -> usize {
    let Some(GuardrailValue::Array(messages)) = data.get_mut("messages") else {
        return 0;
    };
    for (message, redaction) in messages.iter_mut().zip(redacted) {
        if let (GuardrailValue::Object(object), Some(content)) =
            (message, redaction.content.as_ref())
        {
            object.insert("content".to_owned(), content.clone());
        }
    }
    messages.len()
}

fn write_extra_sources(
    data: &mut BTreeMap<String, GuardrailValue>,
    sources: &[ExtraSource],
    redacted: &[RedactedMessage],
    mut offset: usize,
) -> Result<(), CatoError> {
    for source in sources {
        let end = offset.saturating_add(source.messages.len());
        // LiteLLM slices past the end to a short batch instead of dropping the field
        let remaining = redacted.get(offset..).unwrap_or_default();
        let slice = remaining.get(..source.messages.len()).unwrap_or(remaining);
        offset = end;
        match source.field {
            ExtraField::Input => {
                let rewritten = data
                    .get("input")
                    .cloned()
                    .and_then(|input| redacted_input(&input, slice))
                    .ok_or(CatoError::RedactionMismatch)?;
                data.insert("input".to_owned(), rewritten);
            }
            ExtraField::Instructions => {
                if let Some(content) = slice.first().and_then(|message| message.content.as_ref()) {
                    data.insert("instructions".to_owned(), content.clone());
                }
            }
            ExtraField::Prompt => apply_prompt_redaction(data, slice),
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
    // LiteLLM joins the redacted contents, which empties a string input when Cato redacted none of it
    if matches!(input, GuardrailValue::Text(_))
        && redacted.iter().all(|message| message.content.is_none())
    {
        return Some(input.clone());
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
) {
    let Some(prompt) = data.get("prompt").cloned() else {
        return;
    };
    let contents = redacted
        .iter()
        .map(|message| message.content.as_ref())
        .collect::<Vec<_>>();
    let rewritten = match prompt {
        GuardrailValue::Text(_) => contents.first().and_then(|first| *first).cloned(),
        GuardrailValue::Array(parts) => Some(GuardrailValue::Array(rewrite_prompt_parts(
            parts, &contents,
        ))),
        _ => None,
    };
    if let Some(value) = rewritten {
        data.insert("prompt".to_owned(), value);
    }
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
                *part = (*content).clone();
            }
        }
    }
    rewritten
}
