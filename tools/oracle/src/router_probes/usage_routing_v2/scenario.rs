use std::io;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_read_batch::{
    ReadTiers, RequestReads, RoutingPrefetch, RoutingReadKeys, UsageReadKeys,
};
use betterlite_router::routing_value_store::RouteValue;
use betterlite_router::usage_routing_v2::error::{
    KeyError, LimitError, NoDeploymentAvailable, SelectionError,
};
use betterlite_router::usage_routing_v2::selection::CounterKeys;
use betterlite_router::usage_routing_v2::service::UsageRoutingV2;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::runtime::Builder;

use crate::router_probes::random::InjectedRandom;

use self::store::{Clock, ScenarioStore, at, new_tier, seed_tier};
use super::{DeploymentInput, ErrorShape, Input, Output, Step, StepKind, StepOutput};

mod store;

pub(super) fn run(input: &Input, random: &mut InjectedRandom<'_>) -> Result<Output, ScenarioError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| ScenarioError::Runtime { source })?;
    runtime.block_on(run_inner(input, random))
}

async fn run_inner(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<Output, ScenarioError> {
    let mut scenario = Scenario::new(input)?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(scenario.run_step(step, random).await?);
    }
    Ok(Output {
        steps,
        reads: scenario.store.reads(),
    })
}

// the adapter's own failures, each holding the error it came from until the probe boundary renders it
#[derive(Debug, Error)]
pub(super) enum ScenarioError {
    #[error("the step time {millis} milliseconds is not representable")]
    Time { millis: u64 },
    #[error("a deployment is invalid")]
    Deployment {
        #[from]
        source: InvalidDeployment,
    },
    #[error("a routing key is invalid")]
    Key {
        #[from]
        source: KeyError,
    },
    #[error(transparent)]
    Limit {
        #[from]
        source: LimitError,
    },
    #[error("request {request} is not one of the scenario's requests")]
    UnknownRequest { request: usize },
    #[error("the scenario has no usage tier")]
    NoUsageTier,
    #[error("the scenario runtime could not start")]
    Runtime {
        #[source]
        source: io::Error,
    },
}

struct Scenario {
    group: String,
    deployments: Vec<DeploymentSpec>,
    clock: Clock,
    tiers: ReadTiers<RouteValue>,
    store: ScenarioStore,
    requests: Vec<RequestReads<RouteValue>>,
}

impl Scenario {
    fn new(input: &Input) -> Result<Self, ScenarioError> {
        let mut deployments = Vec::with_capacity(input.deployments.len());
        let mut pairs = Vec::with_capacity(input.deployments.len());
        for deployment in &input.deployments {
            deployments.push(build_deployment(&input.model_group, deployment)?);
            pairs.push((deployment.id.clone(), deployment.model.clone()));
        }
        let clock = Clock::new();
        if let Some(first) = input.steps.first() {
            *clock.seconds.lock().unwrap_or_else(PoisonError::into_inner) =
                Duration::from_millis(first.now_millis).as_secs_f64();
        }
        let clock_source: Arc<dyn Fn() -> f64 + Send + Sync> = {
            let seconds = Arc::clone(&clock.seconds);
            Arc::new(move || *seconds.lock().unwrap_or_else(PoisonError::into_inner))
        };
        let usage = new_tier(Arc::clone(&clock_source));
        seed_tier(&usage, &input.local)?;
        let tiers = ReadTiers {
            cooldown: Arc::new(new_tier(clock_source)),
            usage: Some(Arc::new(usage)),
        };
        let count = input
            .steps
            .iter()
            .map(|step| step.request)
            .max()
            .map_or(1, |request| request.saturating_add(1));
        Ok(Self {
            group: input.model_group.clone(),
            deployments,
            clock,
            tiers,
            store: ScenarioStore::new(pairs, input.counters.clone()),
            requests: (0..count).map(|_| RequestReads::default()).collect(),
        })
    }

    async fn run_step(
        &mut self,
        step: &Step,
        random: &mut InjectedRandom<'_>,
    ) -> Result<StepOutput, ScenarioError> {
        *self
            .clock
            .seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Duration::from_millis(step.now_millis).as_secs_f64();
        match step.kind {
            StepKind::Arm => {
                let armed = self.arm(step)?;
                Ok(StepOutput {
                    kind: StepKind::Arm,
                    armed: Some(armed),
                    selected: None,
                    error: None,
                })
            }
            StepKind::SelectAsync => self.select_async(step, random).await,
            StepKind::SelectSync => self.select_sync(step, random),
            StepKind::Down | StepKind::Up => {
                self.store.set_down(step.kind == StepKind::Down);
                Ok(StepOutput {
                    kind: step.kind,
                    armed: None,
                    selected: None,
                    error: None,
                })
            }
        }
    }

    fn arm(&mut self, step: &Step) -> Result<bool, ScenarioError> {
        let keys = CounterKeys::for_deployments(&self.deployments, at(step.now_millis)?)?;
        let read_keys = RoutingReadKeys {
            cooldown: Vec::new(),
            usage: Some(UsageReadKeys {
                tpm: keys.tpm,
                rpm: keys.rpm,
            }),
        };
        let request = self
            .requests
            .get_mut(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?;
        Ok(RoutingPrefetch::arm(&read_keys, &self.tiers, true, request))
    }

    async fn select_async(
        &mut self,
        step: &Step,
        random: &mut InjectedRandom<'_>,
    ) -> Result<StepOutput, ScenarioError> {
        let selection = usage(&self.group, &self.deployments, step)?;
        let request = self
            .requests
            .get_mut(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?;
        let outcome = selection
            .select_async(&self.store, &self.tiers, request, random)
            .await;
        self.observation(StepKind::SelectAsync, outcome)
    }

    fn select_sync(
        &self,
        step: &Step,
        random: &mut InjectedRandom<'_>,
    ) -> Result<StepOutput, ScenarioError> {
        let tier = self.tiers.usage.clone().ok_or(ScenarioError::NoUsageTier)?;
        let selection = usage(&self.group, &self.deployments, step)?;
        self.observation(
            StepKind::SelectSync,
            selection.select_now(Some(tier.as_ref()), random),
        )
    }

    fn observation(
        &self,
        kind: StepKind,
        outcome: Result<usize, SelectionError>,
    ) -> Result<StepOutput, ScenarioError> {
        match outcome {
            Ok(index) => Ok(StepOutput {
                kind,
                armed: None,
                selected: self
                    .deployments
                    .get(index)
                    .and_then(|deployment| deployment.id())
                    .map(str::to_owned),
                error: None,
            }),
            Err(SelectionError::NoDeployment(reported)) => Ok(StepOutput {
                kind,
                armed: None,
                selected: None,
                error: Some(shape(&reported)),
            }),
            Err(SelectionError::Key(source)) => Err(ScenarioError::Key { source }),
            Err(SelectionError::Limit(source)) => Err(ScenarioError::Limit { source }),
        }
    }
}

// the selection one step makes: the pool, the request's token estimate and the step's time
fn usage<'model>(
    group: &'model str,
    deployments: &'model [DeploymentSpec],
    step: &Step,
) -> Result<UsageRoutingV2<'model>, ScenarioError> {
    Ok(UsageRoutingV2 {
        group,
        deployments,
        input_tokens: step.input_tokens,
        now: at(step.now_millis)?,
    })
}

fn shape(reported: &NoDeploymentAvailable) -> ErrorShape {
    ErrorShape {
        status_code: Some(NoDeploymentAvailable::STATUS_CODE),
        code: Some(NoDeploymentAvailable::CODE.to_owned()),
        error_type: Some(NoDeploymentAvailable::ERROR_TYPE.to_owned()),
        retry_after: Some(NoDeploymentAvailable::RETRY_AFTER_SECONDS.to_string()),
        message: reported.to_string(),
    }
}

// one deployment dict of the pinned model_list, its limits in the sources the selector reads
fn build_deployment(group: &str, input: &DeploymentInput) -> Result<DeploymentSpec, ScenarioError> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(input.model.clone()));
    insert(&mut params, "tpm", input.params_tpm.as_ref());
    insert(&mut params, "rpm", input.params_rpm.as_ref());
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(input.id.clone()));
    insert(&mut info, "tpm", input.info_tpm.as_ref());
    insert(&mut info, "rpm", input.info_rpm.as_ref());
    let mut extra = JsonObject::new();
    insert(&mut extra, "tpm", input.tpm.as_ref());
    insert(&mut extra, "rpm", input.rpm.as_ref());
    Ok(DeploymentSpec::new(
        group.to_owned(),
        params,
        info,
        extra,
        String::new,
    )?)
}

fn insert(map: &mut JsonObject, field: &str, value: Option<&Json>) {
    if let Some(found) = value {
        map.insert(field.to_owned(), found.clone());
    }
}
