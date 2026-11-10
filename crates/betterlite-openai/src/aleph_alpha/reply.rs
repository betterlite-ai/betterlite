use std::collections::BTreeMap;
use std::str;

use betterlite_tokenizer::engine::Engine;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::aleph_alpha::error::AlephAlphaError;
use crate::chat::choice_finish_reason;
use crate::chat::response::{AssistantMessage, ChatChoice, ChatUsage, MessageProviderFields};

pub fn decode_aleph_reply(
    status_code: u16,
    body: &[u8],
) -> Result<Vec<ChatChoice>, AlephAlphaError> {
    // LiteLLM's response.json() raises on anything but a JSON object; betterlite keeps the typed error and the reply status
    let Ok(fields) = serde_json::from_slice::<BTreeMap<String, &RawValue>>(body) else {
        return Err(provider_error(status_code, body));
    };
    if let Some(error) = fields.get("error") {
        // a string error gives its own text, any other value the exact bytes of the reply
        let message = serde_json::from_str::<String>(error.get())
            .unwrap_or_else(|_source| error.get().to_owned());
        return Err(AlephAlphaError::Provider {
            status_code,
            message,
        });
    }
    let Some(Value::Array(completions)) = fields
        .get("completions")
        .and_then(|raw| serde_json::from_str(raw.get()).ok())
    else {
        return Err(provider_error(status_code, body));
    };
    let mut choices = Vec::with_capacity(completions.len());
    for (index, item) in completions.iter().enumerate() {
        choices.push(choice_entry(status_code, body, index, item)?);
    }
    Ok(choices)
}

pub fn resolve_usage(
    prompt: &str,
    choices: &[ChatChoice],
    tokenizer: &Engine,
) -> Result<ChatUsage, AlephAlphaError> {
    let prompt_tokens = count_tokens(prompt, tokenizer)?;
    let first = choices.first().ok_or(AlephAlphaError::NoChoices)?;
    // a null completion encodes as no tokens instead of raising the TypeError tiktoken raises on None
    let completion_tokens = count_tokens(
        first.message.content.as_deref().unwrap_or_default(),
        tokenizer,
    )?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(AlephAlphaError::Usage)?;
    Ok(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    })
}

fn count_tokens(text: &str, tokenizer: &Engine) -> Result<u64, AlephAlphaError> {
    let tokens = tokenizer
        .encode(text)
        .map_err(|source| AlephAlphaError::Tokenize { source })?;
    u64::try_from(tokens.len()).map_err(|_source| AlephAlphaError::Usage)
}

fn choice_entry(
    status_code: u16,
    body: &[u8],
    index: usize,
    item: &Value,
) -> Result<ChatChoice, AlephAlphaError> {
    let Value::Object(fields) = item else {
        return Err(provider_error(status_code, body));
    };
    let Some(Value::String(completion)) = fields.get("completion") else {
        return Err(provider_error(status_code, body));
    };
    let Some(finish) = fields.get("finish_reason") else {
        return Err(provider_error(status_code, body));
    };
    // LiteLLM numbers the choices from one
    let position = u32::try_from(index)
        .ok()
        .and_then(|position| position.checked_add(1))
        .ok_or_else(|| provider_error(status_code, body))?;
    let (reason, native) = mapped_finish_reason(finish, status_code, body)?;
    Ok(ChatChoice {
        index: position,
        message: AssistantMessage {
            role: String::from("assistant"),
            content: (!completion.is_empty()).then(|| completion.clone()),
            tool_calls: None,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason: Some(reason),
        logprobs: None,
        provider_specific_fields: native,
    })
}

// Choices maps the raw value and keeps it as native_finish_reason when the mapping changed it
fn mapped_finish_reason(
    value: &Value,
    status_code: u16,
    body: &[u8],
) -> Result<(String, BTreeMap<String, Value>), AlephAlphaError> {
    let raw = match value {
        Value::String(text) => Some(text.as_str()),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
        Value::Array(_) | Value::Object(_) => return Err(provider_error(status_code, body)),
    };
    Ok(choice_finish_reason(raw))
}

fn provider_error(status_code: u16, body: &[u8]) -> AlephAlphaError {
    let message = match str::from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(_source) => String::from("the reply body is not valid UTF-8"),
    };
    AlephAlphaError::Provider {
        status_code,
        message,
    }
}
