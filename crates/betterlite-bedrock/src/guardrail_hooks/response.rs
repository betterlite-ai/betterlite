use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunks::StreamChunk;

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

pub(crate) fn response_field<'response>(
    response: &'response GuardrailValue,
    key: &str,
) -> Option<&'response GuardrailValue> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    fields
        .get(key)
        .filter(|value| !matches!(value, GuardrailValue::Null))
}

// A replacement response reports the model the exception named, which only falls back to the assembled one for raw streams.
#[must_use]
pub fn response_model(response: &GuardrailValue) -> Option<&str> {
    let Some(GuardrailValue::Text(model)) = response_field(response, "model") else {
        return None;
    };
    Some(model.as_str())
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
        streaming_choices.push(streaming_choice(choice)?);
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

fn streaming_choice(choice: &GuardrailValue) -> Option<GuardrailValue> {
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
    if let Some(logprobs) = choice_fields
        .get("logprobs")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        fields.insert("logprobs".to_owned(), logprobs.clone());
    }
    Some(GuardrailValue::Object(fields))
}

fn choices(response: &GuardrailValue) -> Option<&[GuardrailValue]> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Array(items)) = fields.get("choices") else {
        return None;
    };
    Some(items.as_slice())
}
