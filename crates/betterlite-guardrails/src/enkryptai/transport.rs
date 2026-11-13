use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use super::error::EnkryptAiError;
use super::wire::{DetectPayload, DetectionResponse};
use super::{API_PATH, EnkryptAiGuardrail};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) enum Parsed {
    Response(DetectionResponse),
    Malformed(EnkryptAiError),
}

pub(crate) struct Response {
    pub(crate) raw: GuardrailValue,
    pub(crate) parsed: Parsed,
}

pub(crate) async fn send(
    guardrail: &EnkryptAiGuardrail,
    text: &str,
) -> Result<Response, EnkryptAiError> {
    let url = format!("{}{}", guardrail.settings.api_base, API_PATH);
    let payload = DetectPayload {
        text: text.to_owned(),
    };
    let response = guardrail
        .client
        .post(url)
        .headers(guardrail.request_headers()?)
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
    let body = read_capped(response).await?;
    let raw: GuardrailValue =
        serde_json::from_slice(&body).map_err(|source| EnkryptAiError::ResponseJson { source })?;
    if !matches!(raw, GuardrailValue::Object(_)) {
        return Ok(Response {
            raw,
            parsed: Parsed::Malformed(EnkryptAiError::MalformedResponse),
        });
    }
    let value =
        serde_json::to_value(&raw).map_err(|source| EnkryptAiError::Serialize { source })?;
    let parsed = match serde_json::from_value::<DetectionResponse>(value) {
        Ok(response) => Parsed::Response(response),
        Err(source) => Parsed::Malformed(EnkryptAiError::ResponseShape { source }),
    };
    Ok(Response { raw, parsed })
}

impl EnkryptAiGuardrail {
    pub(crate) fn request_headers(&self) -> Result<HeaderMap, EnkryptAiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key =
            HeaderValue::from_str(self.settings.api_key.expose_secret()).map_err(|source| {
                EnkryptAiError::InvalidHeader {
                    name: "apikey",
                    source,
                }
            })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("apikey"), api_key);
        if let Some(policy) = &self.settings.policy_name {
            let value =
                HeaderValue::from_str(policy).map_err(|source| EnkryptAiError::InvalidHeader {
                    name: "x-enkrypt-policy",
                    source,
                })?;
            headers.insert(HeaderName::from_static("x-enkrypt-policy"), value);
        }
        Ok(headers)
    }
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
