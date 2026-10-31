use super::{DeepKeepError, DeepKeepGuardrail, Request, Response};

pub(super) enum Body {
    Json(Response),
    Text(String),
}

pub(super) async fn send_request(
    guardrail: &DeepKeepGuardrail,
    request: &Request,
) -> Result<(reqwest::StatusCode, Body), DeepKeepError> {
    let response = guardrail
        .client
        .post(&guardrail.settings.api_base)
        .headers(guardrail.request_headers()?)
        .json(request)
        .timeout(guardrail.timeout)
        .send()
        .await
        .map_err(|source| {
            if source.is_timeout() {
                DeepKeepError::Timeout { source }
            } else if source.is_builder() {
                DeepKeepError::RequestBuild { source }
            } else {
                DeepKeepError::Request { source }
            }
        })?;
    let status = response.status();
    let body = if status.is_success() {
        Body::Json(
            response
                .json()
                .await
                .map_err(|source| DeepKeepError::Request { source })?,
        )
    } else {
        Body::Text(
            response
                .text()
                .await
                .map_err(|source| DeepKeepError::Request { source })?,
        )
    };
    Ok((status, body))
}
