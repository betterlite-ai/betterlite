use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
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

impl AktoMirroring {
    pub async fn send_request(
        &self,
        flags: AktoCallFlags,
        payload: &AktoPayload,
    ) -> Result<AktoAnswer, AktoError> {
        let endpoint = format!("{}{HTTP_PROXY_PATH}", self.base_url);
        let body = serde_json::to_string(payload).map_err(|source| AktoError::Encode { source })?;
        let response = self
            .client
            .post(endpoint)
            .query(&build_query_params(flags))
            .headers(prepare_headers(&self.api_key)?)
            .timeout(self.timeout)
            .body(body)
            .send()
            .await
            .map_err(|source| AktoError::Transport { source })?;
        let status = response.status();
        let body = read_body(response).await?;
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
        let parsed: GuardrailValue = serde_json::from_slice(&answer.body).map_err(|source| {
            tracing::error!(
                bytes = answer.body.len(),
                "Akto returned a non-JSON body for status 200"
            );
            AktoError::ResponseJson { source }
        })?;
        verdict_from(&parsed)
    }

    pub async fn fire_and_forget_request(&self, flags: AktoCallFlags, payload: &AktoPayload) {
        match self.send_request(flags, payload).await {
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

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, AktoError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| AktoError::Transport { source })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AktoError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
