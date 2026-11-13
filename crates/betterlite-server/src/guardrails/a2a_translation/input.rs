use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

use super::{A2aTranslationError, paths};

// LiteLLM writes back through the parts list it read before the call
const INPUT_PARTS_PATH: [&str; 3] = ["params", "message", "parts"];

#[derive(Debug, Default, PartialEq, Eq)]
struct TextParts {
    texts: Vec<String>,
    indices: Vec<usize>,
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), A2aTranslationError>
where
    G: GuardrailContract,
{
    let Some((message, parts)) = scannable_parts(data) else {
        tracing::debug!("A2A: No parts in message, skipping guardrail");
        return Ok(());
    };
    if parts.texts.is_empty() {
        return Ok(());
    }
    let structured = structured_message(message);
    let model = non_empty_text(data.get("model"));
    let inputs = GuardrailApiInputs {
        texts: Some(parts.texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: Some(vec![structured]),
        model,
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await?;
    let Some(returned) = guardrailed.texts else {
        return Ok(());
    };
    if returned.len() != parts.indices.len() {
        return Ok(());
    }
    let path = INPUT_PARTS_PATH.map(str::to_owned);
    for (part_index, text) in parts.indices.iter().zip(returned.iter()) {
        paths::apply_text_to_path(data, &path, *part_index, text);
    }
    tracing::debug!(
        "A2A: Processed input message with {} text parts",
        parts.indices.len()
    );
    Ok(())
}

fn scannable_parts(
    data: &BTreeMap<String, GuardrailValue>,
) -> Option<(&BTreeMap<String, GuardrailValue>, TextParts)> {
    let Some(GuardrailValue::Object(params)) = data.get("params") else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = params.get("message") else {
        return None;
    };
    let Some(GuardrailValue::Array(parts)) = message.get("parts") else {
        return None;
    };
    if parts.is_empty() {
        return None;
    }
    Some((message, collect_text_parts(parts)))
}

fn collect_text_parts(parts: &[GuardrailValue]) -> TextParts {
    let mut collected = TextParts::default();
    for (index, part) in parts.iter().enumerate() {
        let GuardrailValue::Object(fields) = part else {
            continue;
        };
        if !matches!(fields.get("kind"), Some(GuardrailValue::Text(kind)) if kind == "text") {
            continue;
        }
        let Some(GuardrailValue::Text(text)) = fields.get("text") else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        collected.texts.push(text.clone());
        collected.indices.push(index);
    }
    collected
}

// LiteLLM passes the message through unread, so a message without a text role still gets scanned
fn structured_message(message: &BTreeMap<String, GuardrailValue>) -> GuardrailMessage {
    let mut extra = message.clone();
    let role = match extra.remove("role") {
        Some(GuardrailValue::Text(role)) => role,
        Some(_) | None => String::new(),
    };
    GuardrailMessage { role, extra }
}

fn non_empty_text(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

mod tests;
