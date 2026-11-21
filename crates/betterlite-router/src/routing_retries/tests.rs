#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::future::pending;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::error::details::{GuardrailDetails, RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use rand::{Rng, TryRng};
use tokio::join;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use crate::retry_policy::backoff::BackoffSettings;
use crate::retry_policy::eligibility::Availability;
use crate::retry_policy::policy::RetryPolicies;
use crate::router_core::error::RoutingError;
use crate::routing_retries::attempt::{
    AttemptDispatch, AttemptFailure, AttemptRequest, ProviderFailure,
};
use crate::routing_retries::run::{Retried, RetryRun, run};
use crate::routing_timeouts::plan::TimeoutPlan;
use crate::routing_timeouts::seconds::Seconds;

mod routing_retries_behaviors;
mod routing_retries_boundaries;

pub(super) const GROUP: &str = "gpt-4o";

// every draw is 0, Python's random() of the sample 0, so each backoff has no jitter
pub(super) struct ZeroDraws {
    pub draws: usize,
}

impl TryRng for ZeroDraws {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        self.draws = self.draws.saturating_add(1);
        Ok(0)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        self.draws = self.draws.saturating_add(1);
        Ok(0)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        dst.fill(0);
        Ok(())
    }
}

pub(super) enum Step {
    Answer(&'static str),
    Fail(&'static str, ErrorClass),
    // the caller disconnects while this attempt fails
    Disconnect(&'static str, ErrorClass),
    Unroutable,
    Hang,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Call {
    pub skipped: Vec<String>,
    pub attempted_retries: i64,
    pub max_retries: i64,
    pub breadcrumbs: usize,
}

pub(super) struct Scripted {
    pub steps: VecDeque<Step>,
    pub calls: Vec<Call>,
    pub availability: VecDeque<Availability>,
    pub deployment_num_retries: Option<u32>,
    pub routing: CancellationToken,
    pub attempt_tokens: Vec<CancellationToken>,
}

#[async_trait]
impl AttemptDispatch for Scripted {
    type Response = &'static str;

    async fn attempt<R>(
        &mut self,
        request: AttemptRequest<'_>,
        _random: &mut R,
    ) -> Result<&'static str, AttemptFailure>
    where
        R: Rng + Send,
    {
        self.attempt_tokens.push(request.cancellation.clone());
        self.calls.push(Call {
            skipped: request.skipped_deployment_ids.to_vec(),
            attempted_retries: request.trail.attempted_retries,
            max_retries: request.trail.max_retries,
            breadcrumbs: request.trail.previous_models.len(),
        });
        let failure = |deployment: &str, error: ErrorClass| {
            AttemptFailure::Provider(Box::new(ProviderFailure {
                error,
                deployment_id: Some(deployment.to_owned()),
                deployment_num_retries: self.deployment_num_retries,
                retry_after: None,
            }))
        };
        match self.steps.pop_front().unwrap() {
            Step::Answer(body) => Ok(body),
            Step::Fail(deployment, error) => Err(failure(deployment, error)),
            Step::Disconnect(deployment, error) => {
                self.routing.cancel();
                Err(failure(deployment, error))
            }
            Step::Unroutable => Err(AttemptFailure::Routing(Box::new(RoutingError::Cancelled))),
            Step::Hang => pending().await,
        }
    }

    async fn availability(&mut self) -> Result<Availability, RoutingError> {
        if self.availability.len() > 1 {
            return Ok(self.availability.pop_front().unwrap());
        }
        Ok(*self.availability.front().unwrap())
    }
}

pub(super) struct Scenario {
    pub steps: Vec<Step>,
    pub router_num_retries: i64,
    pub request_num_retries: Option<i64>,
    pub deployment_num_retries: Option<u32>,
    pub policies: RetryPolicies,
    pub availability: Vec<Availability>,
    pub deadline: Duration,
    // the caller disconnects this long after the request starts
    pub cancel_after: Option<Duration>,
}

impl Scenario {
    pub(super) fn new(steps: Vec<Step>, router_num_retries: i64) -> Self {
        Self {
            steps,
            router_num_retries,
            request_num_retries: None,
            deployment_num_retries: None,
            policies: RetryPolicies::default(),
            availability: vec![availability(1, 1)],
            deadline: Duration::from_secs(30),
            cancel_after: None,
        }
    }
}

pub(super) struct Driven {
    pub retried: Retried<&'static str>,
    pub elapsed: Duration,
    pub calls: Vec<Call>,
    pub attempt_tokens: Vec<CancellationToken>,
    pub draws: usize,
}

pub(super) async fn drive(scenario: Scenario) -> Driven {
    let started = Instant::now();
    let now = started.into_std();
    let plan = TimeoutPlan {
        deadline: now.checked_add(scenario.deadline).unwrap(),
        attempt: Seconds(600.0),
        stream: None,
        cancellation: CancellationToken::new(),
    };
    let mut scripted = Scripted {
        steps: VecDeque::from(scenario.steps),
        calls: Vec::new(),
        availability: VecDeque::from(scenario.availability),
        deployment_num_retries: scenario.deployment_num_retries,
        routing: plan.cancellation.clone(),
        attempt_tokens: Vec::new(),
    };
    let settings = RetryRun {
        model_group: GROUP,
        model_group_size: 2,
        request_num_retries: scenario.request_num_retries,
        router_num_retries: scenario.router_num_retries,
        policies: &scenario.policies,
        backoff: BackoffSettings {
            initial_retry_delay: 0.5,
            max_retry_delay: 8.0,
            jitter: 0.75,
            min_timeout: 0.0,
        },
        plan: &plan,
    };
    let mut random = ZeroDraws { draws: 0 };
    let disconnect = async {
        if let Some(after) = scenario.cancel_after {
            sleep(after).await;
            plan.cancellation.cancel();
        }
    };
    let (retried, ()) = join!(run(&mut scripted, &settings, &mut random), disconnect);
    Driven {
        retried,
        elapsed: started.elapsed(),
        calls: scripted.calls,
        attempt_tokens: scripted.attempt_tokens,
        draws: random.draws,
    }
}

pub(super) fn availability(healthy: usize, all: usize) -> Availability {
    Availability {
        healthy_deployments: healthy,
        all_deployments: all,
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        regular_fallbacks: None,
    }
}

pub(super) fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

pub(super) fn rate_limit(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(message),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

pub(super) fn internal(message: &str) -> ErrorClass {
    ErrorClass::InternalServer(Box::new(context(message)))
}

pub(super) fn bad_request(message: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
        context(message),
        Some(400),
    )))
}

pub(super) fn authentication(message: &str) -> ErrorClass {
    ErrorClass::Authentication(Box::new(ResponseDetails::with_status(
        context(message),
        Some(401),
    )))
}

pub(super) fn guardrail(message: &str) -> ErrorClass {
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: None,
        message: message.to_owned(),
        status_code: 400,
        blocked_content: true,
    }))
}

pub(super) fn millis(values: &[u64]) -> Vec<Duration> {
    values
        .iter()
        .map(|value| Duration::from_millis(*value))
        .collect()
}
