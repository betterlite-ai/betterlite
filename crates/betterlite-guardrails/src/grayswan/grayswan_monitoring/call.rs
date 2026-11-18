use betterlite_config::sections::grayswan::Configured;
use reqwest::{Client, Response};

use super::super::error::{GraySwanFailure, GraySwanGuardrailAPIError};
use super::super::wire::{GraySwanPayload, GraySwanResponse};
use super::headers::prepare_headers;

pub const MONITOR_PATH: &str = "/cygnal/monitor";
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

// LiteLLM's _call_grayswan_api: post the payload, raise for a non-2xx, wrap every failure with its status code
pub async fn call_grayswan_api(
    client: &Client,
    settings: &Configured,
    payload: &GraySwanPayload,
) -> Result<GraySwanResponse, GraySwanGuardrailAPIError> {
    let endpoint = format!("{}{MONITOR_PATH}", settings.api_base.trim_end_matches('/'));
    let response = client
        .post(endpoint)
        .headers(prepare_headers(settings)?)
        .json(payload)
        .timeout(settings.guardrail_timeout)
        .send()
        .await
        .map_err(request_failure)?;
    let status = response.status();
    // LiteLLM's raise_for_status(): every non-2xx response is an error, a 3xx included
    if !status.is_success() {
        return Err(GraySwanGuardrailAPIError::from_failure(
            GraySwanFailure::HttpStatus { status },
        ));
    }
    let body = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
    let response: GraySwanResponse = serde_json::from_slice(&body).map_err(|source| {
        GraySwanGuardrailAPIError::from_failure(GraySwanFailure::ResponseJson { source })
    })?;
    tracing::debug!(response = ?response, "Gray Swan Guardrail: monitor response");
    Ok(response)
}

fn request_failure(source: reqwest::Error) -> GraySwanGuardrailAPIError {
    let failure = if source.is_timeout() {
        GraySwanFailure::Timeout { source }
    } else {
        GraySwanFailure::Request { source }
    };
    GraySwanGuardrailAPIError::from_failure(failure)
}

async fn read_capped_body(
    mut response: Response,
    limit: usize,
) -> Result<Vec<u8>, GraySwanGuardrailAPIError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_failure)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(GraySwanGuardrailAPIError::from_failure(
                GraySwanFailure::ResponseTooLarge { limit },
            ));
        }
    }
    Ok(body)
}
