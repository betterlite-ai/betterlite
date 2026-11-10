use thiserror::Error;

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
    settings_timeout: f64,
) -> Result<f64, TimeoutError> {
    let stream_levels = [
        ("kwargs.stream_timeout", request.stream_timeout.as_ref()),
        (
            "litellm_params.stream_timeout",
            deployment.stream_timeout.as_ref(),
        ),
        ("router_stream_timeout", router.stream_timeout.as_ref()),
    ];
    let levels = [
        ("kwargs.timeout", request.timeout.as_ref()),
        ("kwargs.request_timeout", request.request_timeout.as_ref()),
        ("litellm_params.timeout", deployment.timeout.as_ref()),
        (
            "litellm_params.request_timeout",
            deployment.request_timeout.as_ref(),
        ),
        ("router_timeout", router.timeout.as_ref()),
    ];
    let winner = request
        .stream
        .then_some(stream_levels)
        .into_iter()
        .flatten()
        .chain(levels)
        .find_map(|(field, value)| value.map(|set| (field, set)));
    match winner {
        Some((field, value)) => value.seconds(field),
        None => Ok(configured_request_timeout.unwrap_or(settings_timeout)),
    }
}

impl TimeoutValue {
    // only the winning value is validated, so a bad value in a lower field never fails the call
    fn seconds(&self, field: &'static str) -> Result<f64, TimeoutError> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(text) => {
                parse_seconds_text(text).ok_or_else(|| TimeoutError::InvalidSeconds {
                    field,
                    value: text.clone(),
                })
            }
        }
    }
}
