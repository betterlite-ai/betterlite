use std::future::Future;
use std::sync::Arc;

use crate::cooldown_decisions::decision::{
    CooldownDecision, CooldownRequest, DeploymentFacts, MinuteSamples, decide,
};
use crate::cooldown_decisions::failure::{FailureFacts, Seconds};
use crate::cooldown_decisions::policy::CooldownPolicy;
use crate::cooldown_policy::counter::AllowedFailsStore;
use crate::cooldown_policy::error::CooldownPolicyError;
use crate::cooldown_policy::event::{CooldownEvent, CooldownObserver, notify};
use crate::cooldown_policy::facts::error_status_code;
use crate::cooldown_policy::prepared::{
    PreparedCooldown, prepare_callback, prepare_fallback, prepare_set,
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

// one _set_cooldown_deployments call: the deployment, its failed attempt and the cooldown time already resolved
pub struct CooldownCall<'failure> {
    pub deployment_id: &'failure str,
    pub failure: &'failure FailureFacts,
    pub exception: &'failure str,
    pub time_to_cooldown: Option<Seconds>,
    pub requested_model_group: Option<&'failure str>,
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
    pub const fn new(
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
    pub const fn counters(&self) -> &RouterCounters<S> {
        &self.counters
    }

    // Router.deployment_callback_on_failure: the minute failure lands first, then the cooldown decision runs
    pub fn deployment_callback_on_failure<'call>(
        &'call self,
        lookup: &CatalogLookup<'_>,
        attempt: &CooldownAttempt<'_>,
    ) -> impl Future<Output = Result<bool, CooldownPolicyError>> + Send + 'call {
        let prepared = prepare_callback(&self.counters, &self.settings, lookup, attempt);
        async move {
            match prepared? {
                Some(prepared) => self.finish_cooldown(&prepared).await,
                None => Ok(false),
            }
        }
    }

    // _trigger_cooldown_for_failed_deployment: best effort, so a failure here never breaks the fallback response itself
    pub fn trigger_for_failed_deployment<'call>(
        &'call self,
        lookup: &CatalogLookup<'_>,
        failure: &FallbackFailure<'_>,
    ) -> impl Future<Output = bool> + Send + 'call {
        let prepared = prepare_fallback(&self.counters, &self.settings, lookup, failure);
        async move {
            let cooled = match prepared {
                Ok(Some(prepared)) => self.finish_cooldown(&prepared).await,
                Ok(None) => return false,
                Err(error) => Err(error),
            };
            match cooled {
                Ok(cooled) => cooled,
                Err(error) => {
                    tracing::debug!("Error triggering cooldown for fallback deployment: {error}");
                    false
                }
            }
        }
    }

    // _set_cooldown_deployments: B369 decides, the entry lands in B307's cache, and the event fires once per cooldown
    pub fn set_cooldown_deployments<'call>(
        &'call self,
        lookup: &CatalogLookup<'_>,
        call: &CooldownCall<'_>,
    ) -> impl Future<Output = Result<bool, CooldownPolicyError>> + Send + 'call {
        let prepared = prepare_set(&self.counters, lookup, call);
        async move {
            match prepared? {
                Some(prepared) => self.finish_cooldown(&prepared).await,
                None => Ok(false),
            }
        }
    }

    async fn finish_cooldown(
        &self,
        prepared: &PreparedCooldown,
    ) -> Result<bool, CooldownPolicyError> {
        let decision = self
            .decide_with_counter(
                &prepared.facts,
                &prepared.failure,
                prepared.minute,
                prepared.time_to_cooldown,
            )
            .await;
        if !decision.cooldown {
            return Ok(false);
        }
        self.cooldowns
            .add(
                &prepared.deployment_id,
                &prepared.exception,
                error_status_code(&prepared.failure.status),
                prepared.time_to_cooldown.map(|seconds| seconds.0),
            )
            .map_err(|source| CooldownPolicyError::Cooldown {
                model_id: prepared.deployment_id.clone(),
                source,
            })?;
        let event = CooldownEvent {
            model_name: &prepared.event.model_name,
            model_id: &prepared.event.model_id,
            api_base: &prepared.event.api_base,
            provider: &prepared.event.provider,
            exception_status: &prepared.event.exception_status,
        };
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
            policy: request.policy,
            deployment: request.deployment,
            failure: request.failure,
            minute: request.minute,
            allowed_fails_before: observed.fails.saturating_sub(1),
            local_fails: observed.local_only.then_some(observed.fails),
            time_to_cooldown: request.time_to_cooldown,
        };
        decide(&counted)
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
