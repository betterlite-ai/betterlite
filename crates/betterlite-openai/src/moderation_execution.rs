use std::time::{Duration, Instant};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::{JsonPost, UpstreamResponse};
use secrecy::SecretString;
use thiserror::Error;

use crate::chat::deployment::{ChatDeployment, DeploymentError, Provider};
use crate::chat::execution::ChatExecutor;
use crate::moderation_request::{ModerationRequest, ModerationRequestError};
use crate::moderation_response::{ModerationResponse, ModerationResponseError};

// LiteLLM's _get_openai_client passes DEFAULT_MAX_RETRIES, two unless the environment overrides it, and the SDK's initial retry delay
const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Error)]
pub enum ModerationError {
    #[error(transparent)]
    Request(#[from] ModerationRequestError),
    #[error(transparent)]
    Response(#[from] ModerationResponseError),
    #[error("the moderation endpoint could not be built: {source}")]
    Endpoint {
        #[source]
        source: DeploymentError,
    },
    #[error("the provider could not be reached: {source}")]
    Http {
        #[source]
        source: HttpError,
    },
    #[error("the provider answered with status {}", .response.status)]
    Provider { response: Box<UpstreamResponse> },
    #[error("betterlite does not serve {model} moderation yet")]
    UnsupportedProvider { model: String },
    #[error("no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY")]
    MissingApiKey,
}

#[derive(Debug)]
pub struct ModerationOutcome {
    pub response: ModerationResponse,
    pub upstream: UpstreamResponse,
    pub upstream_time: Duration,
}

pub async fn send(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ModerationRequest,
) -> Result<ModerationOutcome, ModerationError> {
    let (model, authorization) = servable(deployment)?;
    let mut attempts = 0;
    loop {
        let post = post(deployment, request, model, authorization.clone())?;
        let started = Instant::now();
        let upstream = executor
            .client
            .post_json(post)
            .await
            .map_err(|source| ModerationError::Http { source })?;
        if upstream.status >= 400 {
            if attempts < MAX_RETRIES && retryable(upstream.status) {
                tokio::time::sleep(retry_delay(attempts)).await;
                attempts = attempts.saturating_add(1);
                continue;
            }
            return Err(ModerationError::Provider {
                response: Box::new(upstream),
            });
        }
        let upstream_time = started.elapsed();
        let response = ModerationResponse::parse(&upstream.body)?;
        return Ok(ModerationOutcome {
            response,
            upstream,
            upstream_time,
        });
    }
}

// openai's _should_retry without a telling response header: request timeouts, lock timeouts, rate limits and 5xx retry
const fn retryable(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500..=599)
}

// openai's _calculate_retry_timeout: the initial delay doubled per retry
const fn retry_delay(attempts: u32) -> Duration {
    INITIAL_RETRY_DELAY.saturating_mul(1 << attempts)
}

fn post(
    deployment: &ChatDeployment,
    request: &ModerationRequest,
    model: &str,
    authorization: SecretString,
) -> Result<JsonPost, ModerationError> {
    let url = deployment
        .endpoint_for("moderations")
        .map_err(|source| ModerationError::Endpoint { source })?;
    let body = request.upstream_body(model)?;
    Ok(JsonPost {
        url,
        authorization,
        headers: Vec::new(),
        body,
        timeout: deployment.timeout,
        body_limit: 0x0200_0000,
    })
}

fn servable(deployment: &ChatDeployment) -> Result<(&str, SecretString), ModerationError> {
    match &deployment.provider {
        Provider::OpenAi { model } => deployment
            .credential()
            .map(|authorization| (model.as_str(), authorization))
            .ok_or(ModerationError::MissingApiKey),
        Provider::Unsupported { model } => Err(ModerationError::UnsupportedProvider {
            model: model.clone(),
        }),
    }
}

mod tests;
