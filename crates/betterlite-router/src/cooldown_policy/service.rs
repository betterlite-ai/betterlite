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
use crate::cooldown_policy::facts::{deployment_facts, error_status_code};
use crate::cooldown_policy::prepared::{
    PreparedCooldown, prepare_callback, prepare_cooldown, prepare_fallback,
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

// the two production entries that share the Router callback's pipeline: a failed attempt and the direct set call
#[derive(Clone, Copy)]
pub enum CooldownSource<'failure> {
    Callback(&'failure CooldownAttempt<'failure>),
    Direct(&'failure CooldownCall<'failure>),
}

// Router's cooldown services: the cache B307 keeps entries in, the minute counters B304 keeps, the shared allowed-fails counters and the observers
pub struct CooldownPolicyService<S: DeltaStore + 'static> {
    settings: CooldownPolicy,
    pub counters: RouterCounters<S>,
    cooldowns: CooldownCache,
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

    // Router.deployment_callback_on_failure and _set_cooldown_deployments: the guards, the minute failure and the decision
    pub fn cool_deployment<'call>(
        &'call self,
        lookup: &CatalogLookup<'_>,
        source: CooldownSource<'_>,
    ) -> impl Future<Output = Result<bool, CooldownPolicyError>> + Send + 'call {
        let prepared = prepare(&self.counters, &self.settings, lookup, source);
        async move {
            let Some(prepared) = prepared? else {
                return Ok(false);
            };
            self.finish_cooldown(&prepared).await
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
            match prepared {
                Ok(Some(prepared)) => {
                    self.finish_cooldown(&prepared)
                        .await
                        .unwrap_or_else(|error| {
                            tracing::debug!(
                                "Error triggering cooldown for fallback deployment: {error}"
                            );
                            false
                        })
                }
                Ok(None) => false,
                Err(error) => {
                    tracing::debug!("Error triggering cooldown for fallback deployment: {error}");
                    false
                }
            }
        }
    }

    // the sync prelude names the counter, the store counts, and the second decide applies B369's thresholds to that count
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

    // _async_get_cooldown_deployments and _get_cooldown_deployments: the same read reduced to the cooling ids
    pub async fn cooling_deployment_ids(
        &self,
        batch: &mut RoutingReadBatch<CooldownRecord>,
        request: &mut RequestReads<CooldownRecord>,
        model_ids: &[String],
    ) -> Result<Vec<String>, CooldownPolicyError> {
        let mut ids = Vec::new();
        for cooldown in self.cooldown_debug_info(batch, request, model_ids).await? {
            ids.push(cooldown.model_id);
        }
        Ok(ids)
    }

    // _async_get_cooldown_deployments_with_debug_info: today's entries that are still cooling
    pub async fn cooldown_debug_info(
        &self,
        batch: &mut RoutingReadBatch<CooldownRecord>,
        request: &mut RequestReads<CooldownRecord>,
        model_ids: &[String],
    ) -> Result<Vec<ActiveCooldown>, CooldownPolicyError> {
        self.cooldowns
            .read(batch, request, model_ids)
            .await
            .map_err(CooldownPolicyError::from)
            .and_then(|results| {
                self.cooldowns
                    .active_from_results(model_ids, &results)
                    .map_err(CooldownPolicyError::from)
            })
    }
}

fn prepare<S: DeltaStore + 'static>(
    counters: &RouterCounters<S>,
    settings: &CooldownPolicy,
    lookup: &CatalogLookup<'_>,
    source: CooldownSource<'_>,
) -> Result<Option<PreparedCooldown>, CooldownPolicyError> {
    match source {
        CooldownSource::Callback(attempt) => prepare_callback(counters, settings, lookup, attempt),
        CooldownSource::Direct(call) => {
            let Some(deployment) = lookup.get_deployment(call.deployment_id)? else {
                return Ok(None);
            };
            let facts = deployment_facts(
                lookup,
                deployment,
                call.deployment_id,
                call.requested_model_group,
            )?;
            prepare_cooldown(counters, deployment, call, facts).map(Some)
        }
    }
}
