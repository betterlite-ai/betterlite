use std::sync::Arc;

use crate::cooldown_decisions::decision::{
    CooldownDecision, CooldownRequest, DeploymentFacts, MinuteSamples, decide,
};
use crate::cooldown_decisions::failure::{
    FailureFacts, Seconds, is_background_cost_poll_404, is_caller_timeout_408,
};
use crate::cooldown_decisions::policy::CooldownPolicy;
use crate::cooldown_policy::counter::AllowedFailsStore;
use crate::cooldown_policy::error::CooldownPolicyError;
use crate::cooldown_policy::event::{CooldownObserver, notify};
use crate::cooldown_policy::facts::{
    cooldown_event, deployment_facts, error_status_code, error_status_text,
    resolve_time_to_cooldown,
};
use crate::cooldown_state::cooldown::{ActiveCooldown, CooldownCache};
use crate::cooldown_state::record::CooldownRecord;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::routing_read_batch::{RequestReads, RoutingReadBatch};
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::DeltaStore;

// the deployment_callback_on_failure call: the failed attempt, the exception it carries and the response header time
pub struct CooldownAttempt<'failure> {
    pub deployment_id: &'failure str,
    pub failure: &'failure FailureFacts,
    pub exception: &'failure str,
    pub header_retry_after: Option<Seconds>,
    pub requested_model_group: Option<&'failure str>,
}

// the _trigger_cooldown_for_failed_deployment call: only the server-stamped failed_deployment_id is trusted
pub struct FallbackFailure<'failure> {
    pub deployment_id: Option<&'failure str>,
    pub failure: &'failure FailureFacts,
    pub exception: &'failure str,
    pub original_generic_function: bool,
    pub header_retry_after: Option<Seconds>,
}

// Router's cooldown services: the cache B307 keeps entries in, the minute counters B304 keeps, the shared
// allowed-fails counters and the observers a cooldown event reaches
pub struct CooldownPolicyService<S: DeltaStore + 'static> {
    settings: CooldownPolicy,
    cooldowns: CooldownCache,
    counters: RouterCounters<S>,
    fails: AllowedFailsStore,
    observers: Vec<Arc<dyn CooldownObserver>>,
}

impl<S: DeltaStore + 'static> CooldownPolicyService<S> {
    #[must_use]
    pub fn new(
        settings: CooldownPolicy,
        cooldowns: CooldownCache,
        counters: RouterCounters<S>,
        fails: AllowedFailsStore,
        observers: Vec<Arc<dyn CooldownObserver>>,
    ) -> Self {
        Self {
            settings,
            cooldowns,
            counters,
            fails,
            observers,
        }
    }

    // the minute counters the decision reads, so a test or caller can record a success the success callback would
    #[must_use]
    pub fn counters(&self) -> &RouterCounters<S> {
        &self.counters
    }

    // Router.deployment_callback_on_failure: the minute failure lands first, then the cooldown decision runs
    pub async fn deployment_callback_on_failure(
        &self,
        lookup: &CatalogLookup<'_>,
        attempt: &CooldownAttempt<'_>,
    ) -> Result<bool, CooldownPolicyError> {
        let Some(deployment) = lookup.get_deployment(attempt.deployment_id)? else {
            return Ok(false);
        };
        let failure = attempt.failure;
        if failure.advisor_orchestration
            || is_background_cost_poll_404(&failure.status, failure.background_response_cost_poll)
            || is_caller_timeout_408(&failure.status, failure.caller_timeout.as_ref())
        {
            return Ok(false);
        }
        let time_to_cooldown = resolve_time_to_cooldown(
            deployment,
            attempt.header_retry_after,
            self.settings.cooldown_time,
        )?;
        self.increment_failures(attempt.deployment_id)?;
        self.set_cooldown_deployments(
            lookup,
            attempt.deployment_id,
            failure,
            attempt.exception,
            Some(time_to_cooldown),
            attempt.requested_model_group,
        )
        .await
    }

    // _trigger_cooldown_for_failed_deployment: best effort, so a failure here never breaks the fallback response itself
    pub async fn trigger_for_failed_deployment(
        &self,
        lookup: &CatalogLookup<'_>,
        failure: &FallbackFailure<'_>,
    ) -> bool {
        match self.try_trigger(lookup, failure).await {
            Ok(cooled) => cooled,
            Err(error) => {
                tracing::debug!("Error triggering cooldown for fallback deployment: {error}");
                false
            }
        }
    }

    async fn try_trigger(
        &self,
        lookup: &CatalogLookup<'_>,
        failure: &FallbackFailure<'_>,
    ) -> Result<bool, CooldownPolicyError> {
        let facts = failure.failure;
        if facts.advisor_orchestration {
            return Ok(false);
        }
        // a generic API call's caller-supplied id can 404 on its own, so that status is not a health signal
        if failure.original_generic_function && error_status_code(&facts.status) == 404 {
            return Ok(false);
        }
        if is_caller_timeout_408(&facts.status, facts.caller_timeout.as_ref()) {
            return Ok(false);
        }
        let Some(deployment_id) = failure.deployment_id else {
            return Ok(false);
        };
        let Some(deployment) = lookup.get_deployment(deployment_id)? else {
            return Ok(false);
        };
        let time_to_cooldown = resolve_time_to_cooldown(
            deployment,
            failure.header_retry_after,
            self.settings.cooldown_time,
        )?;
        self.increment_failures(deployment_id)?;
        self.set_cooldown_deployments(
            lookup,
            deployment_id,
            facts,
            failure.exception,
            Some(time_to_cooldown),
            None,
        )
        .await
    }

    // _set_cooldown_deployments: B369 decides, the entry lands in B307's cache, and the event fires once per cooldown
    pub async fn set_cooldown_deployments(
        &self,
        lookup: &CatalogLookup<'_>,
        deployment_id: &str,
        failure: &FailureFacts,
        exception: &str,
        time_to_cooldown: Option<Seconds>,
        requested_model_group: Option<&str>,
    ) -> Result<bool, CooldownPolicyError> {
        let Some(deployment) = lookup.get_deployment(deployment_id)? else {
            return Ok(false);
        };
        let facts = deployment_facts(lookup, deployment, deployment_id, requested_model_group)?;
        let minute = MinuteSamples {
            successes: minute_count(self.counters.successes_for_minute(deployment_id)),
            failures: minute_count(self.counters.failures_for_minute(deployment_id)),
        };
        let decision = self
            .decide_with_counter(&facts, failure, minute, time_to_cooldown)
            .await;
        if !decision.cooldown {
            return Ok(false);
        }
        self.cooldowns
            .add(
                deployment_id,
                exception,
                error_status_code(&failure.status),
                time_to_cooldown.map(|seconds| seconds.0),
            )
            .map_err(|source| CooldownPolicyError::Cooldown {
                model_id: deployment_id.to_owned(),
                source,
            })?;
        let status_text = error_status_text(&failure.status);
        let event = cooldown_event(deployment, deployment_id, &status_text);
        notify(&self.observers, &event);
        Ok(true)
    }

    // should_cooldown_based_on_allowed_fails_policy reads the counter only after _should_run_cooldown_logic passed,
    // and the first decide call is what names that counter, so the count is read between two decisions
    async fn decide_with_counter(
        &self,
        facts: &DeploymentFacts,
        failure: &FailureFacts,
        minute: MinuteSamples,
        time_to_cooldown: Option<Seconds>,
    ) -> CooldownDecision {
        let request = CooldownRequest {
            policy: &self.settings,
            deployment: Some(facts),
            failure,
            minute,
            allowed_fails_before: 0,
            local_fails: None,
            time_to_cooldown,
        };
        let first = decide(&request);
        let Some(counter) = first.counter.clone() else {
            return first;
        };
        let observed = self
            .fails
            .increment(&counter.key, counter.ttl_seconds.0)
            .await;
        let counted = CooldownRequest {
            allowed_fails_before: observed.fails.saturating_sub(1),
            local_fails: observed.local_only.then_some(observed.fails),
            ..request
        };
        decide(&counted)
    }

    fn increment_failures(&self, deployment_id: &str) -> Result<(), CooldownPolicyError> {
        self.counters
            .increment_failures_for_minute(deployment_id)
            .map_err(|source| CooldownPolicyError::Counter {
                model_id: deployment_id.to_owned(),
                source,
            })
    }

    // _async_get_cooldown_deployments_with_debug_info: today's ids and the entries behind them
    pub async fn cooldown_debug_info(
        &self,
        batch: &mut RoutingReadBatch<CooldownRecord>,
        request: &mut RequestReads<CooldownRecord>,
        model_ids: &[String],
    ) -> Result<Vec<ActiveCooldown>, CooldownPolicyError> {
        let results = self.cooldowns.read(batch, request, model_ids).await?;
        let active = self.cooldowns.active_from_results(model_ids, &results)?;
        Ok(active)
    }

    // _async_get_cooldown_deployments: the same read, reduced to the deployment ids still cooling down
    pub async fn cooldown_ids(
        &self,
        batch: &mut RoutingReadBatch<CooldownRecord>,
        request: &mut RequestReads<CooldownRecord>,
        model_ids: &[String],
    ) -> Result<Vec<String>, CooldownPolicyError> {
        let cooling = self.cooldown_debug_info(batch, request, model_ids).await?;
        Ok(cooling
            .into_iter()
            .map(|cooldown| cooldown.model_id)
            .collect())
    }
}

fn minute_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}
