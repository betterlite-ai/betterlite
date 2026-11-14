use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use super::error::EnkryptAiError;
use super::wire::{DetectPayload, DetectionResponse};
use super::{API_PATH, EnkryptAiGuardrail};
use crate::primitives::json_value::JsonValue;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) enum Parsed {
    Response(Box<DetectionResponse>),
    Malformed(EnkryptAiError),
}

pub(crate) struct Response {
    pub(crate) raw: GuardrailValue,
    pub(crate) parsed: Parsed,
}

pub(crate) async fn send(guardrail: &EnkryptAiGuardrail, text: &str) -> Response {
    let body = match post(guardrail, text).await {
        Ok(body) => body,
        Err(error) => return failed(error),
    };
    let raw = match serde_json::from_slice::<JsonValue>(&body) {
        Ok(raw) => raw,
        Err(source) => return failed(EnkryptAiError::ResponseJson { source }),
    };
    let parsed = match raw {
        JsonValue::Object(_) => match serde_json::from_slice::<DetectionResponse>(&body) {
            Ok(response) => Parsed::Response(Box::new(response)),
            Err(source) => Parsed::Malformed(EnkryptAiError::ResponseShape { source }),
        },
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Parsed::Malformed(EnkryptAiError::MalformedResponse),
    };
    Response {
        raw: guardrail_value(raw),
        parsed,
    }
}

fn failed(error: EnkryptAiError) -> Response {
    Response {
        raw: GuardrailValue::Object(BTreeMap::from([(
            "error".to_owned(),
            GuardrailValue::Text(error.to_string()),
        )])),
        parsed: Parsed::Malformed(error),
    }
}

// GuardrailValue cannot read a float out of JSON when serde_json keeps numbers as text
fn guardrail_value(value: JsonValue) -> GuardrailValue {
    match value {
        JsonValue::Null => GuardrailValue::Null,
        JsonValue::Bool(flag) => GuardrailValue::Boolean(flag),
        JsonValue::Number(number) => number
            .as_i64()
            .map(GuardrailValue::SignedInteger)
            .or_else(|| number.as_u64().map(GuardrailValue::UnsignedInteger))
            .or_else(|| number.as_f64().map(GuardrailValue::Decimal))
            .unwrap_or_else(|| GuardrailValue::Text(number.to_string())),
        JsonValue::String(text) => GuardrailValue::Text(text),
        JsonValue::Array(items) => {
            GuardrailValue::Array(items.into_iter().map(guardrail_value).collect())
        }
        JsonValue::Object(fields) => GuardrailValue::Object(
            fields
                .into_iter()
                .map(|(name, field)| (name, guardrail_value(field)))
                .collect(),
        ),
    }
}

async fn post(guardrail: &EnkryptAiGuardrail, text: &str) -> Result<Vec<u8>, EnkryptAiError> {
    let url = format!("{}{}", guardrail.settings.api_base, API_PATH);
    let payload = DetectPayload {
        text: text.to_owned(),
    };
    let response = guardrail
        .client
        .post(url)
        .headers(request_headers(guardrail)?)
        .json(&payload)
        .timeout(guardrail.settings.timeout)
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(EnkryptAiError::HttpStatus {
            status: status.as_u16(),
        });
    }
    read_capped(response).await
}

fn request_headers(guardrail: &EnkryptAiGuardrail) -> Result<HeaderMap, EnkryptAiError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut api_key =
        HeaderValue::from_str(guardrail.settings.api_key.expose_secret()).map_err(|source| {
            EnkryptAiError::InvalidHeader {
                name: "apikey",
                source,
            }
        })?;
    api_key.set_sensitive(true);
    headers.insert(HeaderName::from_static("apikey"), api_key);
    if let Some(policy) = &guardrail.settings.policy_name {
        let value =
            HeaderValue::from_str(policy).map_err(|source| EnkryptAiError::InvalidHeader {
                name: "x-enkrypt-policy",
                source,
            })?;
        headers.insert(HeaderName::from_static("x-enkrypt-policy"), value);
    }
    Ok(headers)
}

fn request_error(source: reqwest::Error) -> EnkryptAiError {
    if source.is_timeout() {
        EnkryptAiError::Timeout { source }
    } else {
        EnkryptAiError::Request { source }
    }
}

async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, EnkryptAiError> {
    let limit_bytes = u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(EnkryptAiError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(EnkryptAiError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
    }
    Ok(body)
}
