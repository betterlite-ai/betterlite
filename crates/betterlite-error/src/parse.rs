use std::collections::BTreeMap;

use crate::facts::{
    MAX_BODY_BYTES, ProviderErrorBody, ProviderErrorFacts, bounded_body, bounded_text,
};
use serde::de::IgnoredAny;

mod constructors;
mod tests;

use self::constructors::{Constructor, constructor};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("exception {name} requires unavailable constructor arguments")]
    Constructor { name: String },
    #[error("exception {name} has no inspectable signature")]
    Signature { name: String },
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

pub fn extract_exception(
    response: Option<ProviderErrorBody>,
    error_text: &str,
    model: &str,
    provider: &str,
    body: Option<ProviderErrorBody>,
) -> Result<Option<LitellmExceptionFacts>, ParseError> {
    let Some(exception_name) = error_text.split("litellm.").skip(1).find_map(|suffix| {
        let token = suffix
            .split(|character: char| !(character.is_alphanumeric() || character == '_'))
            .next()?;
        let prefix_length = token.rfind("Error")?;
        if prefix_length == 0 {
            return None;
        }
        token.get(..prefix_length.checked_add("Error".len())?)
    }) else {
        return Ok(None);
    };
    let Some(shape) = constructor(exception_name) else {
        return Ok(None);
    };
    let (accepted_response, accepted_body) = match shape {
        Constructor::Response => (response, None),
        Constructor::BodyResponse => (response, body),
        Constructor::Plain => (None, None),
        Constructor::MissingArguments => {
            return Err(ParseError::Constructor {
                name: exception_name.to_owned(),
            });
        }
        Constructor::NoSignature => {
            return Err(ParseError::Signature {
                name: exception_name.to_owned(),
            });
        }
    };
    Ok(Some(LitellmExceptionFacts {
        name: exception_name.to_owned(),
        message: bounded_text(error_text),
        model: bounded_text(model),
        provider: bounded_text(provider),
        response: accepted_response,
        body: accepted_body,
    }))
}

#[must_use]
pub fn get_error_message(error: &ProviderErrorFacts) -> Option<String> {
    let body = error.body.as_ref()?;
    let object = body;
    if let Some(message) = object
        .message
        .as_ref()
        .map(String::as_str)
        .filter(|message| !message.is_empty())
    {
        return Some(bounded_text(message));
    }
    object
        .error
        .as_ref()
        .and_then(|nested| nested.message.as_ref().map(String::as_str))
        .map(bounded_text)
}

#[derive(serde::Deserialize)]
struct CodeBody {
    error: Option<CodeFields>,
}

// serde_json::Number's own Deserialize reads the private map token arbitrary_precision uses for floats, which raw f64/i64 untagged variants cannot
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CodeValue {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Unsupported(IgnoredAny),
}

#[derive(serde::Deserialize)]
struct CodeFields {
    code: Option<CodeValue>,
}

const MAX_SAFE_CODE: f64 = 9_223_372_036_854_776_000.0;
const MIN_SAFE_CODE: f64 = -9_223_372_036_854_776_000.0;

fn truncate_float_code(value: f64) -> Option<i64> {
    let truncated = value.trunc();
    if !(MIN_SAFE_CODE..=MAX_SAFE_CODE).contains(&truncated) {
        return None;
    }
    format!("{truncated:.0}").parse().ok()
}

fn python_int(value: CodeValue) -> Option<i64> {
    match value {
        CodeValue::Bool(true) => Some(1),
        CodeValue::Bool(false) => Some(0),
        CodeValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(truncate_float_code)),
        CodeValue::Text(text) => text.trim().parse().ok(),
        CodeValue::Unsupported(_) => None,
    }
}

#[must_use]
pub fn get_body_error_code(error_text: &str) -> Option<i64> {
    if error_text.len() > MAX_BODY_BYTES {
        return None;
    }
    let body = serde_json::from_str::<CodeBody>(error_text).ok()?;
    let code = body.error?.code?;
    python_int(code)
}

#[must_use]
pub fn get_response_headers(error: &ProviderErrorFacts) -> Option<BTreeMap<String, String>> {
    if let Some(headers) = error.headers.as_ref().filter(|headers| !headers.is_empty()) {
        return Some(headers.clone());
    }
    if let Some(headers) = error
        .response
        .as_ref()
        .map(|response| &response.headers)
        .filter(|headers| !headers.is_empty())
    {
        return Some(headers.clone());
    }
    error.litellm_response_headers.clone()
}

#[must_use]
pub fn accepted_init_kwargs(
    parameters: &[&str],
    candidates: &BTreeMap<String, ProviderErrorBody>,
) -> BTreeMap<String, ProviderErrorBody> {
    let mut accepted = BTreeMap::new();
    for (name, value) in candidates {
        if parameters.contains(&name.as_str()) {
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
                status_code: None,
                status_code_is_synthesized: false,
                message: None,
                headers: None,
                response: None,
                litellm_response_headers: None,
                request: None,
            };
            get_error_message(&facts)
        }),
        body: parsed,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    }
}
