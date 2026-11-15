use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AktoError;
use super::payload::AktoPayload;
use super::{AKTO_CONNECTOR_NAME, AktoMirroring, HTTP_PROXY_PATH};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AktoAnswer {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailVerdict {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AktoCallFlags {
    pub guardrails: bool,
    pub ingest_data: bool,
}

pub(super) fn build_query_params(flags: AktoCallFlags) -> BTreeMap<String, String> {
    let mut params =
        BTreeMap::from([("akto_connector".to_owned(), AKTO_CONNECTOR_NAME.to_owned())]);
    if flags.guardrails {
        params.insert("guardrails".to_owned(), "true".to_owned());
    }
    if flags.ingest_data {
        params.insert("ingest_data".to_owned(), "true".to_owned());
    }
    params
}

pub fn prepare_headers(api_key: &SecretString) -> Result<HeaderMap, AktoError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut credential = HeaderValue::from_str(api_key.expose_secret())
        .map_err(|source| AktoError::InvalidHeader { source })?;
    credential.set_sensitive(true);
    headers.insert(AUTHORIZATION, credential);
    Ok(headers)
}

pub async fn send_request(
    guardrail: &AktoMirroring,
    flags: AktoCallFlags,
    payload: &AktoPayload,
) -> Result<AktoAnswer, AktoError> {
    let endpoint = format!("{}{HTTP_PROXY_PATH}", guardrail.base_url);
    let request_body =
        serde_json::to_string(payload).map_err(|source| AktoError::Encode { source })?;
    let mut response = guardrail
        .client
        .post(endpoint)
        .query(&build_query_params(flags))
        .headers(prepare_headers(&guardrail.api_key)?)
        .timeout(guardrail.timeout)
        .body(request_body)
        .send()
        .await
        .map_err(|source| AktoError::Transport { source })?;
    let status = response.status();
    let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(read_error)?;
    Ok(AktoAnswer { status, body })
}

pub fn handle_guardrail_response(answer: &AktoAnswer) -> Result<GuardrailVerdict, AktoError> {
    if answer.status != StatusCode::OK {
        tracing::error!(
            status = answer.status.as_u16(),
            "Akto returned an unexpected status"
        );
        return Err(AktoError::HttpStatus {
            status: answer.status,
        });
    }
    parse_verdict(&answer.body)
}

pub async fn fire_and_forget_request(
    guardrail: &AktoMirroring,
    flags: AktoCallFlags,
    payload: &AktoPayload,
) {
    match send_request(guardrail, flags, payload).await {
        Ok(answer) if answer.status == StatusCode::OK => {}
        Ok(answer) => tracing::error!(
            status = answer.status.as_u16(),
            "Akto fire-and-forget returned a non-200 status"
        ),
        Err(error) => {
            tracing::error!(error = %error, "Akto fire-and-forget error");
        }
    }
}

// Public so the fuzz target and the corpus replay feed arbitrary bytes through the one parse the transport uses.
pub fn parse_verdict(body: &[u8]) -> Result<GuardrailVerdict, AktoError> {
    let parsed: GuardrailValue = serde_json::from_slice(body).map_err(|source| {
        tracing::error!(
            bytes = body.len(),
            "Akto returned a non-JSON body for status 200"
        );
        AktoError::ResponseJson { source }
    })?;
    verdict_from(&parsed)
}

fn verdict_from(value: &GuardrailValue) -> Result<GuardrailVerdict, AktoError> {
    let allowed_default = || GuardrailVerdict {
        allowed: true,
        reason: String::new(),
    };
    let GuardrailValue::Object(result) = value else {
        return Ok(allowed_default());
    };
    let Some(GuardrailValue::Object(data)) = result.get("data") else {
        return Ok(allowed_default());
    };
    let Some(GuardrailValue::Object(guardrails)) = data.get("guardrailsResult") else {
        return Ok(allowed_default());
    };
    let allowed = match guardrails.get("Allowed") {
        None | Some(GuardrailValue::Null) => true,
        Some(GuardrailValue::Boolean(flag)) => *flag,
        Some(_) => {
            return Err(AktoError::ResponseField {
                field: "data.guardrailsResult.Allowed",
            });
        }
    };
    let reason = match guardrails.get("Reason") {
        None | Some(GuardrailValue::Null) => String::new(),
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(_) => {
            return Err(AktoError::ResponseField {
                field: "data.guardrailsResult.Reason",
            });
        }
    };
    Ok(GuardrailVerdict { allowed, reason })
}

fn read_error(error: ReadError) -> AktoError {
    match error {
        ReadError::TooLarge { limit } => AktoError::ResponseTooLarge { limit },
        ReadError::Failed(source) => AktoError::Transport { source },
    }
}
