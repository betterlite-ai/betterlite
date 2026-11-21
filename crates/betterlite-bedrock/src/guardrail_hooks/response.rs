use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunks::StreamChunk;

// LiteLLM's model_response_text: the assistant text of a chat response, used to tell whether the guardrail rewrote it.
#[must_use]
pub fn model_response_text(response: &GuardrailValue) -> String {
    let Some(choices) = choices(response) else {
        return String::new();
    };
    let mut text = String::new();
    for choice in choices {
        let Some(GuardrailValue::Object(message)) = message_object(choice) else {
            continue;
        };
        if let Some(GuardrailValue::Text(content)) = message.get("content") {
            text.push_str(content);
        }
    }
    text
}

// LiteLLM scans only a ModelResponse that carries text; a tool-only response has no content and must skip Bedrock.
#[must_use]
pub fn response_has_text(response: &GuardrailValue) -> Option<bool> {
    let choices = choices(response)?;
    let mut has_text = false;
    for choice in choices {
        let GuardrailValue::Object(choice_fields) = choice else {
            return None;
        };
        let Some(GuardrailValue::Object(message)) = choice_fields.get("message") else {
            return None;
        };
        if let Some(GuardrailValue::Text(content)) = message.get("content") {
            has_text = has_text || !content.is_empty();
        }
    }
    Some(has_text)
}

// A replacement response reports the model the exception named, which only falls back to the assembled one for raw streams.
#[must_use]
pub fn response_model(response: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    match fields.get("model") {
        Some(GuardrailValue::Text(model)) => Some(model.as_str()),
        None | Some(_) => None,
    }
}

// The upstream usage the block reply has to carry, since the replaced response never reaches the client.
#[must_use]
pub fn response_usage(response: &GuardrailValue) -> Option<GuardrailValue> {
    match response {
        GuardrailValue::Object(fields) => fields
            .get("usage")
            .filter(|usage| !matches!(usage, GuardrailValue::Null))
            .cloned(),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

// MockResponseIterator's convert_model_response_to_streaming: the assembled reply replayed as one chat chunk.
#[must_use]
pub fn chat_chunk(response: &GuardrailValue) -> Option<StreamChunk> {
    let GuardrailValue::Object(response_fields) = response else {
        return None;
    };
    let choices = choices(response)?;
    let mut streaming_choices = Vec::with_capacity(choices.len());
    for choice in choices {
        let GuardrailValue::Object(choice_fields) = choice else {
            return None;
        };
        let Some(GuardrailValue::Object(message)) = choice_fields.get("message") else {
            return None;
        };
        let mut delta = BTreeMap::new();
        for (key, value) in message {
            if !matches!(value, GuardrailValue::Null) {
                delta.insert(key.clone(), value.clone());
            }
        }
        let mut fields = BTreeMap::new();
        fields.insert(
            "index".to_owned(),
            choice_fields
                .get("index")
                .cloned()
                .unwrap_or(GuardrailValue::UnsignedInteger(0)),
        );
        fields.insert("delta".to_owned(), GuardrailValue::Object(delta));
        if let Some(finish_reason) = choice_fields
            .get("finish_reason")
            .filter(|value| !matches!(value, GuardrailValue::Null))
        {
            fields.insert("finish_reason".to_owned(), finish_reason.clone());
        }
        streaming_choices.push(GuardrailValue::Object(fields));
    }
    let mut chunk = BTreeMap::new();
    for key in ["id", "created"] {
        if let Some(value) = response_fields
            .get(key)
            .filter(|value| !matches!(value, GuardrailValue::Null))
        {
            chunk.insert(key.to_owned(), value.clone());
        }
    }
    chunk.insert(
        "object".to_owned(),
        GuardrailValue::Text("chat.completion.chunk".to_owned()),
    );
    if let Some(model) = response_fields.get("model") {
        chunk.insert("model".to_owned(), model.clone());
    }
    chunk.insert(
        "choices".to_owned(),
        GuardrailValue::Array(streaming_choices),
    );
    if let Some(usage) = response_fields
        .get("usage")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        chunk.insert("usage".to_owned(), usage.clone());
    }
    Some(StreamChunk::Json(GuardrailValue::Object(chunk)))
}

fn choices(response: &GuardrailValue) -> Option<&[GuardrailValue]> {
    match response {
        GuardrailValue::Object(fields) => match fields.get("choices") {
            Some(GuardrailValue::Array(choices)) => Some(choices),
            None
            | Some(GuardrailValue::Null)
            | Some(GuardrailValue::Boolean(_))
            | Some(GuardrailValue::SignedInteger(_))
            | Some(GuardrailValue::UnsignedInteger(_))
            | Some(GuardrailValue::Decimal(_))
            | Some(GuardrailValue::Text(_))
            | Some(GuardrailValue::Object(_)) => None,
        },
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

fn message_object(choice: &GuardrailValue) -> Option<&GuardrailValue> {
    match choice {
        GuardrailValue::Object(choice_fields) => choice_fields.get("message"),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}
