use std::time::Duration;

use betterlite_error::error::ErrorClass;
use rand::Rng;
use tokio::time::{Instant, sleep, timeout_at};

use crate::retry_policy::backoff::{BackoffSettings, RetryTiming, sleep_before_retry};
use crate::retry_policy::eligibility::{Availability, RetryDecision, decide};
use crate::retry_policy::policy::{RetryCounts, RetryPolicies, plan};
use crate::retry_policy::skip::{FailedDeployment, skipped_deployment_ids};
use crate::routing_retries::attempt::{
    AttemptDispatch, AttemptFailure, AttemptRequest, Observed, ProviderFailure,
};
use crate::routing_retries::error::{RetryError, exhausted, stamp_deployment_retries};
use crate::routing_retries::trail::{RetryAttempt, RetryTrail};
use crate::routing_timeouts::error::TimeoutError;
use crate::routing_timeouts::plan::TimeoutPlan;

// What one request's retries read: the logical group, its counts and policies, the backoff and the routing deadline.
pub struct RetryRun<'run> {
    pub model_group: &'run str,
    pub model_group_size: usize,
    pub request_num_retries: Option<i64>,
    pub router_num_retries: i64,
    pub policies: &'run RetryPolicies,
    pub backoff: BackoffSettings,
    pub plan: &'run TimeoutPlan,
}

// The answer of the whole sequence, with the retry metadata it gathered either way.
#[derive(Debug)]
pub struct Retried<Response> {
    pub outcome: Result<Response, RetryError>,
    pub trail: RetryTrail,
}

// async_function_with_retries: the first attempt, then the retries its failure earns, raising the latest error.
pub async fn run<D, R>(
    dispatch: &mut D,
    settings: &RetryRun<'_>,
    random: &mut R,
) -> Retried<D::Response>
where
    D: AttemptDispatch,
    R: Rng + Send,
{
    let trail = RetryTrail {
        model_group_size: settings.model_group_size,
        attempted_retries: 0,
        max_retries: settings
            .request_num_retries
            .unwrap_or(settings.router_num_retries),
        previous_models: Vec::new(),
        request_retry_count: 0,
        backoffs: Vec::new(),
    };
    let mut attempts = Attempts {
        dispatch,
        settings,
        random,
        trail,
        skipped: Vec::new(),
    };
    let outcome = attempts.execute().await;
    Retried {
        outcome,
        trail: attempts.trail,
    }
}

enum Answer<Response> {
    Committed(Response),
    Failed(ProviderFailure),
}

#[derive(PartialEq, Eq)]
enum Backoff {
    Slept,
    OutOfTime,
}

// What the first failure decided: the count, whether a policy set it, and the deployments it saw.
struct Schedule {
    num_retries: i64,
    policy_applies: bool,
    availability: Availability,
}

// a provider failure earns a retry, while a routing failure ends the loop
fn answer<Response>(
    answered: Result<Response, AttemptFailure>,
) -> Result<Answer<Response>, RetryError> {
    match answered {
        Ok(response) => Ok(Answer::Committed(response)),
        Err(AttemptFailure::Provider(failure)) => {
            Ok(Answer::Failed(stamp_deployment_retries(*failure)))
        }
        Err(AttemptFailure::Routing(error)) => Err(RetryError::Routing(error)),
    }
}

struct Attempts<'state, D, R> {
    dispatch: &'state mut D,
    settings: &'state RetryRun<'state>,
    random: &'state mut R,
    trail: RetryTrail,
    skipped: Vec<String>,
}

impl<D, R> Attempts<'_, D, R>
where
    D: AttemptDispatch,
    R: Rng + Send,
{
    async fn execute(&mut self) -> Result<D::Response, RetryError> {
        let first = match self.first().await? {
            Answer::Committed(response) => return Ok(response),
            Answer::Failed(failure) => failure,
        };
        let Some(schedule) = self.schedule(&first).await? else {
            return Err(RetryError::Provider(Box::new(first.error)));
        };
        self.after_failure(&first);
        let mut latest = first;
        let mut availability = schedule.availability;
        // each wait precedes the attempt it delays, so LiteLLM's last sleep, which nothing follows, never runs
        for attempted in 1..=schedule.num_retries {
            let waited = self
                .backoff(&latest, attempted, schedule.num_retries, &availability)
                .await?;
            if waited == Backoff::OutOfTime {
                return Err(RetryError::Provider(Box::new(latest.error)));
            }
            self.trail.attempted_retries = attempted;
            latest = match self.attempt().await? {
                Answer::Committed(response) => return Ok(response),
                Answer::Failed(failure) => failure,
            };
            self.after_failure(&latest);
            let Some(refreshed) = self.recheck(&latest, &schedule).await? else {
                return Err(RetryError::Provider(Box::new(latest.error)));
            };
            availability = refreshed;
        }
        Err(exhausted(latest.error, schedule.num_retries))
    }

    // the mock switch's rate limit fails the first attempt before any provider call
    async fn first(&mut self) -> Result<Answer<D::Response>, RetryError> {
        if let Err(error) = self.dispatch.mock_rate_limit(self.settings.model_group) {
            return Ok(Answer::Failed(stamp_deployment_retries(ProviderFailure {
                error,
                deployment_id: None,
                deployment_num_retries: None,
                retry_after: None,
            })));
        }
        self.attempt().await
    }

    // the loop's should_retry_this_error, skipped under a policy; None stops the retries
    async fn recheck(
        &mut self,
        latest: &ProviderFailure,
        schedule: &Schedule,
    ) -> Result<Option<Availability>, RetryError> {
        let mut availability = self.dispatch.availability().await.map_err(Box::new)?;
        // LiteLLM refreshes only the healthy deployments and keeps the count the first failure saw
        availability.all_deployments = schedule.availability.all_deployments;
        let observed = Observed::from(&latest.error);
        let decision = decide(observed.class, observed.status, &availability);
        if !schedule.policy_applies && matches!(decision, RetryDecision::Stop(_)) {
            return Ok(None);
        }
        Ok(Some(availability))
    }

    // one attempt under the routing deadline and the caller's cancellation; dropping its future cancels the transport call
    async fn attempt(&mut self) -> Result<Answer<D::Response>, RetryError> {
        let timeouts = self.settings.plan;
        let deadline = timeouts.attempt_deadline(Instant::now().into_std())?;
        let cancellation = timeouts.cancellation.child_token();
        let request = AttemptRequest {
            skipped_deployment_ids: &self.skipped,
            trail: &self.trail,
            deadline,
            cancellation: cancellation.clone(),
        };
        let bounded = timeout_at(
            Instant::from_std(timeouts.deadline),
            self.dispatch.attempt(request, self.random),
        );
        match timeouts.cancellation.run_until_cancelled(bounded).await {
            None => Err(RetryError::Timeout(TimeoutError::Cancelled)),
            Some(Err(_elapsed)) => {
                cancellation.cancel();
                Err(RetryError::Timeout(TimeoutError::DeadlineExpired))
            }
            Some(Ok(answered)) => answer(answered),
        }
    }

    // the first failure decides the count; a guardrail block or an error the check refuses goes straight back
    async fn schedule(&mut self, first: &ProviderFailure) -> Result<Option<Schedule>, RetryError> {
        if matches!(first.error, ErrorClass::GuardrailRaised(_)) {
            return Ok(None);
        }
        let observed = Observed::from(&first.error);
        let counts = RetryCounts {
            request: self.settings.request_num_retries,
            router: self.settings.router_num_retries,
            deployment: first.deployment_num_retries,
        };
        let availability = self.dispatch.availability().await.map_err(Box::new)?;
        let group = Some(self.settings.model_group);
        let planned = plan(
            counts,
            self.settings.policies,
            observed.class,
            observed.status,
            group,
        );
        let decision = decide(observed.class, observed.status, &availability);
        if !planned.policy_applies && matches!(decision, RetryDecision::Stop(_)) {
            return Ok(None);
        }
        self.trail.max_retries = planned.num_retries;
        if planned.num_retries <= 0 {
            return Ok(None);
        }
        Ok(Some(Schedule {
            num_retries: planned.num_retries,
            policy_applies: planned.policy_applies,
            availability,
        }))
    }

    // log_retry, then the skip list: a deployment that answered a status LiteLLM never retries is not picked again
    fn after_failure(&mut self, failure: &ProviderFailure) {
        let observed = Observed::from(&failure.error);
        self.trail.record(RetryAttempt {
            model_group: self.settings.model_group.to_owned(),
            deployment_id: failure.deployment_id.clone(),
            class: observed.class,
            status: observed.status,
            attempted_retries: self.trail.attempted_retries,
        });
        let Some(status) = observed.status else {
            return;
        };
        let failed = FailedDeployment {
            retry_skip_deployment_id: None,
            failed_deployment_id: failure.deployment_id.as_deref(),
        };
        let skipped = skipped_deployment_ids(status, failed, &self.skipped);
        if !skipped.is_empty() {
            self.skipped = skipped;
        }
    }

    // _time_to_sleep_before_retry, refused where the wait would reach the routing deadline
    async fn backoff(
        &mut self,
        failure: &ProviderFailure,
        attempted: i64,
        num_retries: i64,
        availability: &Availability,
    ) -> Result<Backoff, RetryError> {
        // LiteLLM's first wait counts every retry as remaining, and each later one counts one fewer
        let timing = RetryTiming {
            remaining_retries: num_retries.saturating_sub(attempted).saturating_add(1),
            max_retries: num_retries,
        };
        let seconds = sleep_before_retry(
            &self.settings.backoff,
            timing,
            availability,
            failure.retry_after,
            self.random,
        );
        let delay = Duration::try_from_secs_f64(seconds)
            .map_err(|source| RetryError::Backoff { seconds, source })?;
        let timeouts = self.settings.plan;
        if delay >= timeouts.remaining(Instant::now().into_std())? {
            return Ok(Backoff::OutOfTime);
        }
        self.trail.backoffs.push(delay);
        timeouts
            .cancellation
            .run_until_cancelled(sleep(delay))
            .await
            .ok_or(TimeoutError::Cancelled)?;
        Ok(Backoff::Slept)
    }
}
