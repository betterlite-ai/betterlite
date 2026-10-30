use std::collections::BTreeMap;

use crate::facts::{
    ErrorCode, MAX_BODY_BYTES, ProviderErrorBody, ProviderErrorFacts, bounded_body, bounded_text,
};

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("provider error body exceeds the bounded size")]
    BodyTooLarge,
    #[error("provider error body is not a JSON object")]
    BodyNotObject,
    #[error("provider error body is malformed")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LitellmExceptionFacts {
    pub name: String,
    pub message: String,
    pub model: String,
    pub provider: String,
    pub response: Option<ProviderErrorBody>,
    pub body: Option<ProviderErrorBody>,
}

#[must_use]
pub fn extract_exception(
    response: Option<ProviderErrorBody>,
    error_text: &str,
    model: &str,
    provider: &str,
    body: Option<ProviderErrorBody>,
) -> Option<LitellmExceptionFacts> {
    let marker = "litellm.";
    let marker_start = error_text.find(marker)?;
    let suffix = error_text.get(marker_start.checked_add(marker.len())?..)?;
    let name_end = suffix
        .char_indices()
        .find(|(_, character)| !(character.is_ascii_alphanumeric() || *character == '_'))
        .map_or(suffix.len(), |(index, _)| index);
    let exception_name = suffix.get(..name_end)?;
    if !exception_name.ends_with("Error") || exception_name == "Error" {
        return None;
    }
    Some(LitellmExceptionFacts {
        name: exception_name.to_owned(),
        message: bounded_text(error_text),
        model: bounded_text(model),
        provider: bounded_text(provider),
        response,
        body,
    })
}

#[must_use]
pub fn get_error_message(error: &ProviderErrorFacts) -> Option<String> {
    let body = error.body.as_ref()?;
    let object = body;
    if let Some(message) = object
        .message
        .as_deref()
        .filter(|message| !message.is_empty())
    {
        return Some(bounded_text(message));
    }
    object
        .error
        .as_deref()
        .and_then(|nested| nested.message.as_deref())
        .map(bounded_text)
}

#[must_use]
pub fn get_body_error_code(error_text: &str) -> Option<i64> {
    let body = serde_json::from_str::<ProviderErrorBody>(error_text).ok()?;
    let code = body.error.as_deref()?.code.as_ref()?;
    match code {
        ErrorCode::Number(number) => Some(*number),
        ErrorCode::Text(text) => text.parse::<i64>().ok(),
    }
}

#[must_use]
pub fn get_response_headers(error: &ProviderErrorFacts) -> Option<BTreeMap<String, String>> {
    if let Some(headers) = error.headers.as_ref().filter(|headers| !headers.is_empty()) {
        return Some(headers.clone());
    }
    if let Some(headers) = error
        .response_headers
        .as_ref()
        .filter(|headers| !headers.is_empty())
    {
        return Some(headers.clone());
    }
    error
        .litellm_response_headers
        .as_ref()
        .filter(|headers| !headers.is_empty())
        .cloned()
}

#[must_use]
pub fn accepted_init_kwargs(
    candidates: &BTreeMap<String, ProviderErrorBody>,
) -> BTreeMap<String, ProviderErrorBody> {
    let mut accepted = BTreeMap::new();
    for (name, value) in candidates {
        if matches!(name.as_str(), "response" | "body") {
            accepted.insert(name.clone(), value.clone());
        }
    }
    accepted
}

pub fn parse_provider_error(body: &[u8]) -> Result<ProviderErrorBody, ParseError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ParseError::BodyTooLarge);
    }
    let value = serde_json::from_slice::<ProviderErrorBody>(body)
        .map_err(|source| ParseError::InvalidJson { source })?;
    bounded_body(&value).ok_or(ParseError::BodyTooLarge)
}

#[must_use]
pub fn extract_facts(body: &[u8], status_code: Option<u16>) -> ProviderErrorFacts {
    let parsed = parse_provider_error(body).ok();
    ProviderErrorFacts {
        status_code,
        status_code_is_synthesized: status_code.is_none(),
        message: parsed.as_ref().and_then(|value| {
            let facts = ProviderErrorFacts {
                body: Some(value.clone()),
                ..ProviderErrorFacts::empty()
            };
            get_error_message(&facts)
        }),
        body: parsed,
        ..ProviderErrorFacts::empty()
    }
}
