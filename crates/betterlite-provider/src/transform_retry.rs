use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use thiserror::Error;

use crate::operation::OperationError;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};

// LiteLLM's max_retry_on_unprocessable_entity_error defaults to 0, so a provider that never opts in makes one attempt
pub const MAX_TRANSFORM_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformRetry {
    max_attempts: u32,
}

impl TransformRetry {
    pub const DISABLED: Self = Self { max_attempts: 1 };

    pub const fn bounded(max_attempts: u32) -> Result<Self, TransformRetryError> {
        if max_attempts == 0 || max_attempts > MAX_TRANSFORM_ATTEMPTS {
            return Err(TransformRetryError::Attempts { max_attempts });
        }
        Ok(Self { max_attempts })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransformRetryError {
    #[error("the transform retry count {max_attempts} is outside 1..={MAX_TRANSFORM_ATTEMPTS}")]
    Attempts { max_attempts: u32 },
}

// LiteLLM asks the provider config whether a failed call may be retried after the request body is rewritten
pub trait TransformOnUnprocessable: Send + Sync {
    fn should_retry(&self, _response: &UpstreamResponse, _deployment: &Deployment) -> bool {
        false
    }

    fn transform_request(
        &self,
        _prepared: &mut PreparedRequest,
        _response: &UpstreamResponse,
        _deployment: &Deployment,
    ) {
    }
}

pub(crate) async fn send_with_retry(
    client: &ProxiedClient,
    prepared: &PreparedRequest,
    deployment: &Deployment,
    hook: &dyn TransformOnUnprocessable,
) -> Result<Upstream, OperationError> {
    let retry = prepared.retry();
    let mut attempt = 1;
    let mut current = prepared.clone();
    loop {
        let upstream = current.send(client).await?;
        if let Upstream::Buffered(response) = &upstream
            && response.status >= 400
            && attempt < retry.max_attempts
            && hook.should_retry(response, deployment)
        {
            hook.transform_request(&mut current, response, deployment);
            attempt = attempt.saturating_add(1);
            continue;
        }
        return Ok(upstream);
    }
}

mod tests;
