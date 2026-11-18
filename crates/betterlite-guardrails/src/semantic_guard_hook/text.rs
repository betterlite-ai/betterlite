use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// LiteLLM's join raises on a dict part whose text is not a string, and on text it cannot print
#[derive(Debug, thiserror::Error)]
pub enum TextExtractionError {
    #[error("the content part at index {index} carries a text that is not a string")]
    NonTextPart { index: usize },
    #[error("the content part at index {index} could not be written as text")]
    UnrepresentablePart { index: usize },
}

// LiteLLM's _extract_user_text: the last user message whose content is a string or a list
pub(crate) fn user_text(messages: &[GuardrailValue]) -> Result<String, TextExtractionError> {
    for message in messages.iter().rev() {
        let GuardrailValue::Object(fields) = message else {
            continue;
        };
        if !matches!(fields.get("role"), Some(GuardrailValue::Text(role)) if role == "user") {
            continue;
        }
        match fields.get("content") {
            None => return Ok(String::new()),
            Some(GuardrailValue::Text(text)) => return Ok(text.clone()),
            Some(GuardrailValue::Array(parts)) => return parts_text(parts),
            Some(_other) => continue,
        }
    }
    Ok(String::new())
}

fn parts_text(parts: &[GuardrailValue]) -> Result<String, TextExtractionError> {
    let mut texts = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        match part {
            GuardrailValue::Object(fields) => match fields.get("text") {
                None => texts.push(String::new()),
                Some(GuardrailValue::Text(text)) => texts.push(text.clone()),
                Some(_not_text) => return Err(TextExtractionError::NonTextPart { index }),
            },
            other => texts.push(string_form(other, index)?),
        }
    }
    Ok(texts.join(" "))
}

// Python's str of a part that is not a mapping, written in JSON's spelling instead
fn string_form(value: &GuardrailValue, index: usize) -> Result<String, TextExtractionError> {
    match value {
        GuardrailValue::Text(text) => Ok(text.clone()),
        GuardrailValue::Boolean(flag) => Ok(flag.to_string()),
        GuardrailValue::SignedInteger(number) => Ok(number.to_string()),
        GuardrailValue::UnsignedInteger(number) => Ok(number.to_string()),
        GuardrailValue::Decimal(number) => Ok(number.to_string()),
        GuardrailValue::Null => Ok("null".to_owned()),
        GuardrailValue::Array(_) | GuardrailValue::Object(_) => serde_json::to_string(value)
            .map_err(|_not_json| TextExtractionError::UnrepresentablePart { index }),
    }
}

// LiteLLM's _extract_response_text and _content_to_text: every choice's message content, joined by a newline
pub(crate) fn response_text(response: &GuardrailValue) -> String {
    let GuardrailValue::Object(fields) = response else {
        return String::new();
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        return String::new();
    };
    let mut texts = Vec::new();
    for choice in choices {
        let GuardrailValue::Object(choice_fields) = choice else {
            continue;
        };
        let Some(GuardrailValue::Object(message)) = choice_fields.get("message") else {
            continue;
        };
        let Some(content) = message.get("content") else {
            continue;
        };
        let text = content_text(content);
        if !text.is_empty() {
            texts.push(text);
        }
    }
    texts.join("\n")
}

fn content_text(content: &GuardrailValue) -> String {
    match content {
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(parts) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter_map(|part| match part {
                    GuardrailValue::Object(fields) => match fields.get("text") {
                        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.as_str()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            texts.join(" ")
        }
        _ => String::new(),
    }
}
