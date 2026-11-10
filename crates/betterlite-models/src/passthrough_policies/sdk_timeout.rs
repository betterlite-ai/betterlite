use thiserror::Error;

use super::generic_timeout::resolve_passthrough_timeout;

mod seconds_text;
mod tests;

use seconds_text::parse_seconds_text;

#[derive(Debug, Clone, PartialEq)]
pub enum TimeoutValue {
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestTimeouts {
    pub stream: bool,
    pub stream_timeout: Option<TimeoutValue>,
    pub timeout: Option<TimeoutValue>,
    pub request_timeout: Option<TimeoutValue>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeploymentTimeouts {
    pub stream_timeout: Option<TimeoutValue>,
    pub timeout: Option<TimeoutValue>,
    pub request_timeout: Option<TimeoutValue>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouterTimeouts {
    pub stream_timeout: Option<TimeoutValue>,
    pub timeout: Option<TimeoutValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimeoutError {
    #[error("{field} is not a number of seconds: {value}")]
    InvalidSeconds { field: &'static str, value: String },
}

pub fn resolve_sdk_timeout(
    request: &RequestTimeouts,
    deployment: &DeploymentTimeouts,
    router: &RouterTimeouts,
    configured_request_timeout: Option<f64>,
    settings_timeout: Option<f64>,
) -> Result<f64, TimeoutError> {
    let candidates = ordered_candidates(request, deployment, router, configured_request_timeout);
    let Some((field, value)) = candidates.into_iter().next() else {
        return Ok(resolve_passthrough_timeout(None, settings_timeout));
    };
    value.seconds(field)
}

fn ordered_candidates(
    request: &RequestTimeouts,
    deployment: &DeploymentTimeouts,
    router: &RouterTimeouts,
    configured_request_timeout: Option<f64>,
) -> Vec<(&'static str, TimeoutValue)> {
    let mut candidates: Vec<(&'static str, TimeoutValue)> = Vec::with_capacity(9);
    if request.stream {
        push_candidate(
            &mut candidates,
            "kwargs.stream_timeout",
            request.stream_timeout.as_ref(),
        );
        push_candidate(
            &mut candidates,
            "litellm_params.stream_timeout",
            deployment.stream_timeout.as_ref(),
        );
        push_candidate(
            &mut candidates,
            "router_stream_timeout",
            router.stream_timeout.as_ref(),
        );
    }
    push_candidate(&mut candidates, "kwargs.timeout", request.timeout.as_ref());
    push_candidate(
        &mut candidates,
        "kwargs.request_timeout",
        request.request_timeout.as_ref(),
    );
    push_candidate(
        &mut candidates,
        "litellm_params.timeout",
        deployment.timeout.as_ref(),
    );
    push_candidate(
        &mut candidates,
        "litellm_params.request_timeout",
        deployment.request_timeout.as_ref(),
    );
    push_candidate(&mut candidates, "router_timeout", router.timeout.as_ref());
    if let Some(seconds) = configured_request_timeout {
        candidates.push(("litellm.request_timeout", TimeoutValue::Number(seconds)));
    }
    candidates
}

fn push_candidate(
    candidates: &mut Vec<(&'static str, TimeoutValue)>,
    field: &'static str,
    candidate: Option<&TimeoutValue>,
) {
    if let Some(value) = candidate {
        candidates.push((field, value.clone()));
    }
}

impl TimeoutValue {
    // only the winning value is validated, so a bad value in a lower field never fails the call
    fn seconds(self, field: &'static str) -> Result<f64, TimeoutError> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Text(text) => {
                parse_seconds_text(&text).ok_or(TimeoutError::InvalidSeconds { field, value: text })
            }
        }
    }
}
