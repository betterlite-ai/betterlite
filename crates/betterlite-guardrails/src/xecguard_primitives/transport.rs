use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::error::XecGuardError;
use super::message::last_text_by_role;
use super::settings::XecGuardSettings;
use super::wire::{
    GROUNDING_ENDPOINT, GroundingDocument, GroundingPayload, SCAN_ENDPOINT, ScanPayload,
    ScanResult, ScanType, XecGuardMessage,
};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RequestBody<'payload> {
    Scan(&'payload ScanPayload),
    Grounding(&'payload GroundingPayload),
}

impl RequestBody<'_> {
    const fn endpoint(&self) -> &'static str {
        match self {
            Self::Scan(_) => SCAN_ENDPOINT,
            Self::Grounding(_) => GROUNDING_ENDPOINT,
        }
    }

    const fn field_keys(&self) -> &'static [&'static str] {
        match self {
            Self::Scan(_) => &["model", "scan_type", "messages", "policy_names"],
            Self::Grounding(_) => &["model", "prompt", "response", "documents", "strictness"],
        }
    }
}

pub async fn call_scan(
    client: &reqwest::Client,
    settings: &XecGuardSettings,
    messages: &[XecGuardMessage],
    scan_type: ScanType,
    suppress_errors: bool,
) -> Result<Option<ScanResult>, XecGuardError> {
    post(
        client,
        settings,
        &RequestBody::Scan(&scan_payload(settings, messages, scan_type)),
        suppress_errors,
    )
    .await
}

fn scan_payload(
    settings: &XecGuardSettings,
    messages: &[XecGuardMessage],
    scan_type: ScanType,
) -> ScanPayload {
    ScanPayload {
        model: settings.model.clone(),
        scan_type,
        messages: messages.to_vec(),
        policy_names: settings.policy_names.clone(),
    }
}

pub async fn call_grounding(
    client: &reqwest::Client,
    settings: &XecGuardSettings,
    messages: &[XecGuardMessage],
    documents: &[GroundingDocument],
) -> Result<Option<ScanResult>, XecGuardError> {
    let Some(prompt) = last_text_by_role(messages, "user") else {
        return Ok(None);
    };
    let Some(response) = last_text_by_role(messages, "assistant") else {
        return Ok(None);
    };
    let payload = GroundingPayload {
        model: settings.model.clone(),
        prompt,
        response,
        documents: documents.to_vec(),
        strictness: settings.grounding_strictness.clone(),
    };
    post(client, settings, &RequestBody::Grounding(&payload), false).await
}

async fn post(
    client: &reqwest::Client,
    settings: &XecGuardSettings,
    body: &RequestBody<'_>,
    suppress_errors: bool,
) -> Result<Option<ScanResult>, XecGuardError> {
    let endpoint = format!(
        "{}{}",
        settings.api_base.trim_end_matches('/'),
        body.endpoint()
    );
    tracing::debug!(
        endpoint = %endpoint,
        payload_keys = ?body.field_keys(),
        "XecGuard: POST"
    );
    match send(client, settings, &endpoint, body).await {
        Ok(result) => Ok(Some(result)),
        Err(error) => api_failure(settings, suppress_errors, error),
    }
}

fn api_failure(
    settings: &XecGuardSettings,
    suppress_errors: bool,
    error: XecGuardError,
) -> Result<Option<ScanResult>, XecGuardError> {
    tracing::error!(error = %error, "XecGuard API error");
    if suppress_errors {
        return Ok(None);
    }
    if settings.block_on_error {
        return Err(XecGuardError::Unreachable {
            source: Box::new(error),
        });
    }
    Ok(None)
}

async fn send(
    client: &reqwest::Client,
    settings: &XecGuardSettings,
    endpoint: &str,
    body: &RequestBody<'_>,
) -> Result<ScanResult, XecGuardError> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {}", settings.api_key.expose_secret()))
            .map_err(|source| XecGuardError::InvalidHeader { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let response = client
        .post(endpoint)
        .headers(headers)
        .json(body)
        .timeout(settings.timeout)
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    let bytes = match read_capped_body(response).await {
        Err(XecGuardError::ResponseTooLarge { .. }) if !status.is_success() => Vec::new(),
        result => result?,
    };
    if !status.is_success() {
        return Err(XecGuardError::HttpStatus { status });
    }
    serde_json::from_slice(&bytes).map_err(|source| XecGuardError::ResponseJson { source })
}

fn request_error(source: reqwest::Error) -> XecGuardError {
    if source.is_timeout() {
        XecGuardError::Timeout { source }
    } else {
        XecGuardError::Request { source }
    }
}

async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, XecGuardError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(XecGuardError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

mod tests;
