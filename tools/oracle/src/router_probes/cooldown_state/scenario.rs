use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_models::{decisions::Json, model_registration::JsonObject};
use betterlite_router::cooldown_state::cooldown::{ActiveCooldown, CooldownCache};
use betterlite_router::cooldown_state::filter::{HealthRouting, without_unhealthy};
use betterlite_router::cooldown_state::health::{
    DeploymentHealthCache, HealthSnapshot, HealthState,
};
use betterlite_router::cooldown_state::record::{CooldownRecord, KEY_PREFIX, KEY_SUFFIX};
use betterlite_router::cooldown_state::settings::Settings;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_read_batch::{RequestReads, RoutingReadBatch};
use betterlite_router::routing_value_store::{Clock, RouteKey};
use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use tokio::runtime::Builder;

use super::store::{FakeCooldowns, FakeHealth, lock};
use super::{
    ActiveView, CooldownInput, DeploymentInput, HealthInput, Input, Output, ScenarioError, Seconds,
    Step, StepKind, StepOutput,
};

pub(crate) fn run(input: &Input) -> Result<Output, ScenarioError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| ScenarioError::Runtime { source })?;
    runtime.block_on(Scenario::new(input)?.run())
}

type SharedClock = Arc<Mutex<f64>>;

fn clock_fn(clock: &SharedClock) -> Clock {
    let clock = Arc::clone(clock);
    Arc::new(move || *lock(&clock))
}

struct Scenario<'case> {
    input: &'case Input,
    clock: SharedClock,
    cooldowns: CooldownCache,
    health: DeploymentHealthCache,
    deployments: Vec<DeploymentSpec>,
}

impl<'case> Scenario<'case> {
    fn new(input: &'case Input) -> Result<Self, ScenarioError> {
        let clock: SharedClock = Arc::new(Mutex::new(0.0_f64));
        let settings = Settings {
            default_cooldown_time: input.settings.default_cooldown_time.0,
            redis_read_interval_seconds: input.settings.redis_read_interval_seconds.0,
            enable_health_check_routing: input.health.enabled,
            staleness_threshold: input.settings.staleness_threshold.0,
            ignore_transient_errors: false,
        };
        let mut records = BTreeMap::new();
        for (model_id, stored) in &input.shared_cooldowns {
            records.insert(cooldown_key(model_id)?, stored.record());
        }
        let shared = Arc::new(FakeCooldowns {
            records: Mutex::new(records),
        });
        let cooldowns = CooldownCache::new(&settings, shared, clock_fn(&clock));
        let snapshot = snapshot(&input.shared_health);
        let snapshot = (!snapshot.is_empty()).then_some(snapshot);
        let health = DeploymentHealthCache::new(
            settings.staleness_threshold,
            Arc::new(FakeHealth {
                snapshot: Mutex::new(snapshot),
            }),
            clock_fn(&clock),
        )
        .map_err(|_error| ScenarioError::HealthKey)?;
        let deployments = input
            .deployments
            .iter()
            .map(deployment)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            input,
            clock,
            cooldowns,
            health,
            deployments,
        })
    }

    async fn run(self) -> Result<Output, ScenarioError> {
        let mut steps = Vec::with_capacity(self.input.steps.len());
        for step in &self.input.steps {
            *lock(&self.clock) = seconds(step.now_millis)?;
            steps.push(self.run_step(step).await?);
        }
        Ok(Output { steps })
    }

    async fn run_step(&self, step: &Step) -> Result<StepOutput, ScenarioError> {
        let mut output = StepOutput {
            kind: step.kind,
            local_expiry: None,
            local_expiries: None,
            active: None,
            minimum: None,
            unhealthy: None,
            kept: None,
        };
        match step.kind {
            StepKind::Add => output.local_expiry = self.add(step)?,
            StepKind::Read => self.read(step, &mut output).await?,
            StepKind::Minimum => self.minimum(step, &mut output).await?,
            StepKind::SetHealth => output.local_expiry = self.set_health(step)?,
            StepKind::Health => output.unhealthy = Some(self.health.unhealthy_now()),
            StepKind::Filter => output.kept = Some(self.filter(step)),
        }
        Ok(output)
    }

    fn add(&self, step: &Step) -> Result<Option<Seconds>, ScenarioError> {
        self.cooldowns
            .add(
                &step.model_id,
                &step.exception,
                step.status_code,
                step.cooldown_time.map(|time| time.0),
            )
            .map_err(|_error| ScenarioError::CooldownKey)?;
        Ok(self.expiries(&[step.model_id.clone()])?.pop().flatten())
    }

    async fn read(&self, step: &Step, output: &mut StepOutput) -> Result<(), ScenarioError> {
        let results = self.fetched(&step.model_ids).await?;
        let active = self
            .cooldowns
            .active_from_results(&step.model_ids, &results)
            .map_err(|_error| ScenarioError::CooldownKey)?;
        output.active = Some(active.iter().map(active_view).collect());
        output.local_expiries = Some(self.expiries(&step.model_ids)?);
        Ok(())
    }

    async fn minimum(&self, step: &Step, output: &mut StepOutput) -> Result<(), ScenarioError> {
        let results = self.fetched(&step.model_ids).await?;
        output.minimum = Some(Seconds(self.cooldowns.minimum_cooldown(&results)));
        Ok(())
    }

    fn set_health(&self, step: &Step) -> Result<Option<Seconds>, ScenarioError> {
        let states = snapshot(&step.states);
        self.health
            .set_states(&states)
            .map_err(|_error| ScenarioError::HealthState)?;
        Ok(self.health.local_expiry().map(Seconds))
    }

    fn filter(&self, step: &Step) -> Vec<String> {
        let scoped = self
            .input
            .health
            .scoped_groups
            .clone()
            .map(BTreeSet::from_iter);
        let routing = HealthRouting {
            enabled: self.input.health.enabled,
            has_allowed_fails_policy: self.input.health.has_allowed_fails_policy,
            scoped_groups: scoped.as_ref(),
        };
        without_unhealthy(
            self.deployments.clone(),
            &self.health.unhealthy_now(),
            routing,
            step.health_check_probe,
        )
        .iter()
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect()
    }

    async fn fetched(
        &self,
        model_ids: &[String],
    ) -> Result<Vec<Option<CooldownRecord>>, ScenarioError> {
        let mut batch = RoutingReadBatch::default();
        let mut request = RequestReads::default();
        self.cooldowns
            .read(&mut batch, &mut request, model_ids)
            .await
            .map_err(|_error| ScenarioError::CooldownKey)
    }

    fn expiries(&self, model_ids: &[String]) -> Result<Vec<Option<Seconds>>, ScenarioError> {
        Ok(self
            .cooldowns
            .local_expiries(model_ids)
            .map_err(|_error| ScenarioError::CooldownKey)?
            .into_iter()
            .map(|expiry| expiry.map(Seconds))
            .collect())
    }
}

fn snapshot(states: &BTreeMap<String, HealthInput>) -> HealthSnapshot {
    states
        .iter()
        .map(|(model_id, state)| (model_id.clone(), state.state()))
        .collect()
}

fn active_view(cooldown: &ActiveCooldown) -> ActiveView {
    ActiveView {
        model_id: cooldown.model_id.clone(),
        remaining_seconds: Seconds(cooldown.remaining_seconds),
        exception_received: cooldown.record.exception_received.clone(),
        status_code: cooldown.record.status_code.clone(),
        timestamp: Seconds(cooldown.record.timestamp),
        cooldown_time: Seconds(cooldown.record.cooldown_time),
    }
}

impl CooldownInput {
    fn record(&self) -> CooldownRecord {
        CooldownRecord {
            exception_received: self.exception_received.clone(),
            status_code: self.status_code.clone(),
            timestamp: self.timestamp.0,
            cooldown_time: self.cooldown_time.0,
        }
    }
}

impl HealthInput {
    fn state(&self) -> HealthState {
        HealthState {
            is_healthy: self.is_healthy,
            timestamp: self.timestamp.0,
            reason: self.reason.clone(),
        }
    }
}

fn deployment(input: &DeploymentInput) -> Result<DeploymentSpec, ScenarioError> {
    let id = &input.id;
    let params = JsonObject::from([("model".to_owned(), Json::Text(format!("openai/{id}")))]);
    let info = JsonObject::from([("id".to_owned(), Json::Text(id.clone()))]);
    DeploymentSpec::new(
        input.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .map_err(|_error| ScenarioError::Deployment { id: id.clone() })
}

fn cooldown_key(model_id: &str) -> Result<RouteKey, ScenarioError> {
    RouteKey::parse(&format!("{KEY_PREFIX}{model_id}{KEY_SUFFIX}"))
        .map_err(|_error| ScenarioError::CooldownKey)
}

fn seconds(millis: u64) -> Result<f64, ScenarioError> {
    Decimal::from(millis)
        .checked_div(Decimal::from(1_000_i64))
        .and_then(|quotient| quotient.to_f64())
        .ok_or(ScenarioError::TimeOverflow { millis })
}
