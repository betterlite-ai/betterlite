use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_router::cooldown_decisions::decision::{CooldownRequest, DeploymentFacts, decide};
use betterlite_router::cooldown_decisions::failure::Seconds;
use betterlite_router::cooldown_decisions::policy::{CooldownPolicy, FailureThresholds};
use betterlite_router::router_observations::counter::{CounterError, CounterSink, Increment};
use betterlite_router::router_observations::failure::{
    CooldownError, CooldownSink, FailureObservation,
};

use super::input::Payload;

#[derive(Debug, Default)]
pub(super) struct ProbeCounters {
    values: Mutex<BTreeMap<String, i64>>,
}

impl ProbeCounters {
    pub(super) fn value(&self, key: &str) -> Option<i64> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }
}

impl CounterSink for ProbeCounters {
    fn increment(&self, key: &str, value: i64, _ttl: Duration) -> Result<(), CounterError> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let counter = values.entry(key.to_owned()).or_insert(0);
        *counter = counter.saturating_add(value);
        drop(values);
        Ok(())
    }

    fn increment_pipeline(&self, increments: &[Increment]) -> Result<(), CounterError> {
        for increment in increments {
            self.increment(&increment.key, increment.value, increment.ttl)?;
        }
        Ok(())
    }

    fn read(&self, key: &str) -> Option<i64> {
        self.value(key)
    }

    fn write(&self, key: &str, value: i64, _ttl: Option<Duration>) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }
}

// the B308 stand-in: LiteLLM's cooldown cache plus the decision B369 already ports, since this task only delegates to it
pub(super) struct ProbeCooldowns {
    policy: CooldownPolicy,
    deployment: DeploymentFacts,
    active: Mutex<BTreeMap<String, Seconds>>,
    allowed_fails: Mutex<BTreeMap<String, u64>>,
}

impl ProbeCooldowns {
    pub(super) fn new(input: &Payload) -> Self {
        let router = &input.router;
        Self {
            policy: CooldownPolicy {
                allowed_fails: router.allowed_fails,
                default_allowed_fails: router.default_allowed_fails,
                cooldown_time: router.cooldown_time,
                disable_cooldowns: router.disable_cooldowns,
                allowed_fails_policy: router.allowed_fails_policy.clone(),
                thresholds: FailureThresholds::default(),
            },
            deployment: DeploymentFacts {
                id: input.catalog.target_id.clone(),
                model_group_size: input.catalog.group_size,
                has_alternatives: input.catalog.alternatives > 0,
                provider_default: input.deployment.provider_default,
                allowed_fails_policy: None,
                allowed_fails: input.deployment.allowed_fails,
                model_info_cooldown_time: input.deployment.info_cooldown_time,
                litellm_params_cooldown_time: input.deployment.params_cooldown_time,
            },
            active: Mutex::new(BTreeMap::new()),
            allowed_fails: Mutex::new(BTreeMap::new()),
        }
    }

    fn fails_before(&self, key: &str) -> u64 {
        let allowed_fails = self
            .allowed_fails
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        allowed_fails.get(key).copied().unwrap_or_default()
    }

    fn store_fails(&self, key: &str, fails: u64) {
        let mut allowed_fails = self
            .allowed_fails
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        allowed_fails.insert(key.to_owned(), fails);
    }

    pub(super) fn active_time(&self, deployment_id: &str) -> Option<Seconds> {
        let active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        active
            .get(deployment_id)
            .copied()
            .filter(|seconds| seconds.0 > 0.0_f64)
    }
}

impl CooldownSink for ProbeCooldowns {
    fn set_cooldown_deployments(
        &self,
        observation: &FailureObservation<'_>,
    ) -> Result<bool, CooldownError> {
        let mut request = CooldownRequest {
            policy: &self.policy,
            deployment: Some(&self.deployment),
            failure: observation.failure,
            minute: observation.minute,
            allowed_fails_before: 0,
            local_fails: None,
            time_to_cooldown: Some(observation.time_to_cooldown),
        };
        // the decision names the counter it compares against, so the first pass only learns the key
        let draft = decide(&request);
        request.allowed_fails_before = draft
            .counter
            .as_ref()
            .map_or(0, |counter| self.fails_before(&counter.key));
        let decision = decide(&request);
        if let Some(counter) = &decision.counter {
            self.store_fails(&counter.key, counter.fails);
        }
        if decision.cooldown {
            let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            active.insert(
                observation.deployment_id.to_owned(),
                observation.time_to_cooldown,
            );
        }
        Ok(decision.cooldown)
    }
}
