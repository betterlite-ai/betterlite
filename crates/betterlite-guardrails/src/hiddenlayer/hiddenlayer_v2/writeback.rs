use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::hiddenlayer::evaluation::error::EvaluationError;
use crate::hiddenlayer::evaluation::{as_guardrail_message, text_part};

// LiteLLM replaces the messages with the endpoint's evaluation and rebuilds the scanned texts
pub(super) fn write_request(
    inputs: &mut GuardrailApiInputs,
    output: &GuardrailValue,
) -> Result<(), EvaluationError> {
    let GuardrailValue::Object(fields) = output else {
        return Err(EvaluationError::Shape { field: "messages" });
    };
    let messages = match fields.get("messages") {
        Some(GuardrailValue::Array(messages)) => messages.clone(),
        _ => Vec::new(),
    };
    let mut structured = Vec::with_capacity(messages.len());
    let mut texts = Vec::new();
    for message in &messages {
        let converted = as_guardrail_message(message)?;
        if let Some(text) = converted.extra.get("content").and_then(message_text) {
            texts.push(text);
        }
        structured.push(converted);
    }
    inputs.structured_messages = Some(structured);
    inputs.texts = Some(texts);
    Ok(())
}

// A list content joins its text parts with a space; an empty text or no text part adds nothing
fn message_text(content: &GuardrailValue) -> Option<String> {
    match content {
        GuardrailValue::Array(parts) => {
            let texts: Vec<String> = parts.iter().filter_map(text_part).collect();
            (!texts.is_empty()).then(|| texts.join(" "))
        }
        GuardrailValue::Text(text) => (!text.is_empty()).then(|| text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => None,
    }
}

pub(super) fn write_response(
    inputs: &mut GuardrailApiInputs,
    output: &GuardrailValue,
) -> Result<(), EvaluationError> {
    let has_texts = inputs.texts.as_ref().is_some_and(|texts| !texts.is_empty());
    let has_calls = inputs
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty());
    match (has_texts, has_calls) {
        (true, _) => inputs.texts = Some(vec![last_choice_content(output)?]),
        (false, true) => {
            let GuardrailValue::Array(items) = output else {
                return Err(EvaluationError::Shape {
                    field: "tool_calls",
                });
            };
            inputs.tool_calls = Some(items.clone());
        }
        (false, false) => {}
    }
    Ok(())
}

// LiteLLM reads choices[-1]; a missing or null choices, message or content reads as an empty content, an empty list fails
pub(super) fn last_choice_content(output: &GuardrailValue) -> Result<String, EvaluationError> {
    let GuardrailValue::Object(body) = output else {
        return Err(EvaluationError::Shape { field: "choices" });
    };
    let list = match body.get("choices") {
        None | Some(GuardrailValue::Null) => return Ok(String::new()),
        Some(GuardrailValue::Array(list)) => list,
        Some(_) => return Err(EvaluationError::Shape { field: "choices" }),
    };
    let Some(GuardrailValue::Object(choice)) = list.last() else {
        return Err(EvaluationError::Shape { field: "choices" });
    };
    let message = match choice.get("message") {
        Some(GuardrailValue::Null) | None => return Ok(String::new()),
        Some(GuardrailValue::Object(message)) => message,
        Some(_) => {
            return Err(EvaluationError::Shape {
                field: "choices.message",
            });
        }
    };
    match message.get("content") {
        Some(GuardrailValue::Text(content)) => Ok(content.clone()),
        None | Some(GuardrailValue::Null) => Ok(String::new()),
        Some(_) => Err(EvaluationError::Shape {
            field: "choices.message.content",
        }),
    }
}
