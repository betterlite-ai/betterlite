use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::error::TimeoutError;
use super::global::GlobalRequestTimeout;
use super::seconds::Seconds;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RouterTimeoutSources {
    pub timeout: Option<Seconds>,
    pub stream_timeout: Option<Seconds>,
    pub default_timeout: Option<Seconds>,
    pub default_stream_timeout: Option<Seconds>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterTimeouts {
    pub timeout: Seconds,
    pub stream_timeout: Option<Seconds>,
    pub request_timeout: Option<Seconds>,
    pub default_timeout: Option<Seconds>,
    pub default_stream_timeout: Option<Seconds>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RequestTimeouts {
    pub stream: bool,
    pub timeout: Option<Seconds>,
    pub request_timeout: Option<Seconds>,
    pub stream_timeout: Option<Seconds>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeploymentTimeouts {
    pub timeout: Option<Seconds>,
    pub request_timeout: Option<Seconds>,
    pub stream_timeout: Option<Seconds>,
}

// Router.__init__: self.timeout falls back to litellm.request_timeout, and self.request_timeout exists only beside an explicit router timeout
#[must_use]
pub fn resolve_router_timeouts(
    sources: &RouterTimeoutSources,
    global: GlobalRequestTimeout,
) -> RouterTimeouts {
    let request_timeout = sources.timeout.and_then(|_configured| global.configured());
    RouterTimeouts {
        timeout: sources.timeout.unwrap_or(global.seconds),
        stream_timeout: sources.stream_timeout,
        request_timeout,
        default_timeout: sources.default_timeout.or(sources.timeout),
        default_stream_timeout: sources.default_stream_timeout,
    }
}

// the three selections LiteLLM makes separately: _get_stream_timeout, _get_non_stream_timeout and _get_timeout
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectedTimeouts {
    pub stream: Option<Seconds>,
    pub non_stream: Option<Seconds>,
    pub attempt: Option<Seconds>,
}

#[must_use]
pub fn select_timeouts(
    request: &RequestTimeouts,
    deployment: &DeploymentTimeouts,
    router: &RouterTimeouts,
) -> SelectedTimeouts {
    let stream = request
        .stream_timeout
        .or(deployment.stream_timeout)
        .or(router.stream_timeout)
        .or(router.request_timeout)
        .or(router.default_stream_timeout);
    let non_stream = request
        .timeout
        .or(request.request_timeout)
        .or(deployment.timeout)
        .or(deployment.request_timeout)
        .or(router.request_timeout)
        .or(Some(router.timeout))
        .or(router.default_timeout);
    let attempt = if request.stream {
        stream.or(non_stream)
    } else {
        non_stream
    };
    SelectedTimeouts {
        stream,
        non_stream,
        attempt,
    }
}

#[derive(Debug)]
pub struct TimeoutPlan {
    pub deadline: Instant,
    pub attempt: Option<Seconds>,
    pub stream: Option<Seconds>,
    pub cancellation: CancellationToken,
}

impl TimeoutPlan {
    pub fn new(
        now: Instant,
        router: &RouterTimeouts,
        request: &RequestTimeouts,
        deployment: &DeploymentTimeouts,
    ) -> Result<Self, TimeoutError> {
        let whole = select_timeouts(request, &DeploymentTimeouts::default(), router)
            .attempt
            .ok_or(TimeoutError::NoTimeout)?;
        // a deployment change only narrows the attempt; the routing deadline was fixed from the request's own sources
        let selected = select_timeouts(request, deployment, router);
        let deadline = now
            .checked_add(whole.duration("the routing deadline")?)
            .ok_or_else(|| TimeoutError::OutOfRange {
                field: "the routing deadline".to_owned(),
                seconds: whole.0,
            })?;
        Ok(Self {
            deadline,
            attempt: selected.attempt,
            stream: selected.stream,
            cancellation: CancellationToken::new(),
        })
    }

    pub fn attempt_deadline(&self, now: Instant) -> Result<Instant, TimeoutError> {
        if self.cancellation.is_cancelled() {
            return Err(TimeoutError::Cancelled);
        }
        if now >= self.deadline {
            return Err(TimeoutError::DeadlineExpired);
        }
        let Some(attempt) = self.attempt else {
            return Ok(self.deadline);
        };
        let expires = now
            .checked_add(attempt.duration("the attempt timeout")?)
            .ok_or_else(|| TimeoutError::OutOfRange {
                field: "the attempt timeout".to_owned(),
                seconds: attempt.0,
            })?;
        Ok(expires.min(self.deadline))
    }

    pub fn remaining(&self, now: Instant) -> Result<Duration, TimeoutError> {
        if self.cancellation.is_cancelled() {
            return Err(TimeoutError::Cancelled);
        }
        self.deadline
            .checked_duration_since(now)
            .ok_or(TimeoutError::DeadlineExpired)
    }

    // an attempt that fired earlier than the whole routing says whether the deployment or the routing ran out
    #[must_use]
    pub fn expired(&self, now: Instant) -> TimeoutError {
        if self.cancellation.is_cancelled() {
            TimeoutError::Cancelled
        } else if now >= self.deadline {
            TimeoutError::DeadlineExpired
        } else {
            TimeoutError::AttemptExpired
        }
    }
}
