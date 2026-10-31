use std::{iter, str};

use secrecy::ExposeSecret as _;

use super::error::DeepKeepError;
use super::{DeepKeepGuardrail, Request, Response};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) async fn send_request(
    guardrail: &DeepKeepGuardrail,
    request: &Request,
) -> Result<Response, DeepKeepError> {
    let response = guardrail
        .client
        .post(&guardrail.settings.api_base)
        .headers(guardrail.request_headers()?)
        .json(request)
        .timeout(guardrail.timeout)
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    let body = match read_body(response).await {
        // Keep the HTTP status authoritative when the error body exceeds the limit.
        Err(DeepKeepError::ResponseTooLarge { .. }) if !status.is_success() => Vec::new(),
        result => result?,
    };
    if !status.is_success() {
        let message = str::from_utf8(&body)
            .map_err(|source| DeepKeepError::ApiEncoding { status, source })?;
        return Err(DeepKeepError::Api {
            status,
            body: redact_message(guardrail, message),
        });
    }
    let mut parsed: Response =
        serde_json::from_slice(&body).map_err(|source| DeepKeepError::ResponseJson { source })?;
    parsed.blocked_reason = parsed
        .blocked_reason
        .map(|reason| redact_message(guardrail, &reason));
    Ok(parsed)
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, DeepKeepError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(DeepKeepError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn request_error(source: reqwest::Error) -> DeepKeepError {
    if source.is_timeout() {
        DeepKeepError::Timeout { source }
    } else if source.is_builder() {
        DeepKeepError::RequestBuild { source }
    } else {
        DeepKeepError::Request { source }
    }
}

fn redact_message(guardrail: &DeepKeepGuardrail, message: &str) -> String {
    let contains_secret = iter::once(&guardrail.settings.api_key)
        .chain(guardrail.settings.extra_headers.values())
        .any(|secret| {
            let exposed = secret.expose_secret();
            !exposed.is_empty() && message.contains(exposed)
        });
    if contains_secret {
        "[REDACTED]".to_owned()
    } else {
        message.to_owned()
    }
}
