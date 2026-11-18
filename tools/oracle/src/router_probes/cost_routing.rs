mod counter;
mod steps;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cost_routing::costs::{Catalog, ModelCosts};
use betterlite_router::cost_routing::service::CostRouter;
use betterlite_router::cost_routing::window::WindowCounters;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::io_token_reservations::clock::IoClock;
use betterlite_router::routing_state::Settings;
use betterlite_router::routing_state::counter::RouterCounters;
use betterlite_router::routing_state::delta::{Delta, DeltaStore};
use betterlite_router::routing_state::error::RoutingStateError;
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

use self::counter::ProbeText;
use self::steps::ScenarioError;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model_group: String,
    #[serde(default)]
    pub model_cost: BTreeMap<String, JsonObject>,
    pub deployments: Vec<DeploymentInput>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub input_cost_per_token: Option<Json>,
    #[serde(default)]
    pub output_cost_per_token: Option<Json>,
    #[serde(default)]
    pub params_tpm: Option<Json>,
    #[serde(default)]
    pub params_rpm: Option<Json>,
    #[serde(default)]
    pub info_tpm: Option<Json>,
    #[serde(default)]
    pub info_rpm: Option<Json>,
    #[serde(default)]
    pub tpm: Option<Json>,
    #[serde(default)]
    pub rpm: Option<Json>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    #[serde(default)]
    pub now_millis: u64,
    #[serde(default)]
    pub deployment_id: Option<String>,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub total_tokens: Option<i64>,
    #[serde(default)]
    pub input: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Success,
    SuccessSync,
    Select,
    State,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observation>,
}

// one entry per step: the success callbacks write silently, a selection names its deployment, the state shows the map
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Recorded {
        step: usize,
    },
    Selected {
        step: usize,
        deployment_id: Option<String>,
    },
    State {
        step: usize,
        entries: Vec<Found>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
    pub deployment_id: String,
    pub tpm: i64,
    pub rpm: i64,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime
        .block_on(run_scenario(input))
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(ProbeOutput::CostRouting(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

pub(super) struct ProbeCosts<'input> {
    entries: &'input BTreeMap<String, JsonObject>,
}

impl<'input> ProbeCosts<'input> {
    pub(super) const fn new(entries: &'input BTreeMap<String, JsonObject>) -> Self {
        Self { entries }
    }
}

impl ModelCosts for ProbeCosts<'_> {
    fn entry(&self, model: &str) -> Option<Catalog<'_>> {
        self.entries.get(model).map(Catalog::plain)
    }
}

pub(super) struct Context<'run> {
    pub(super) group: &'run str,
    pub(super) deployments: &'run [DeploymentSpec],
    pub(super) costs: &'run ProbeCosts<'run>,
    pub(super) router: &'run CostRouter<WindowCounters<NoSharedStore>>,
    pub(super) text: &'run ProbeText,
    pub(super) clock: &'run Arc<AtomicU64>,
}

pub(super) struct NoSharedStore;

#[async_trait]
impl DeltaStore for NoSharedStore {
    async fn increment_pipeline(
        &self,
        _deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        Ok(Vec::new())
    }
}

async fn run_scenario(input: &Input) -> Result<Output, ScenarioError> {
    let clock = Arc::new(AtomicU64::new(0));
    let router = open_router(&clock)?;
    let mut deployments = Vec::with_capacity(input.deployments.len());
    for deployment in &input.deployments {
        deployments.push(build_deployment(&input.model_group, deployment)?);
    }
    let costs = ProbeCosts::new(&input.model_cost);
    let text = ProbeText::new().map_err(|message| ScenarioError::Tokenizer { message })?;
    let context = Context {
        group: &input.model_group,
        deployments: &deployments,
        costs: &costs,
        router: &router,
        text: &text,
        clock: &clock,
    };
    let mut observed = Vec::with_capacity(input.steps.len());
    for (index, step) in input.steps.iter().enumerate() {
        clock.store(step.now_millis, Ordering::SeqCst);
        observed.push(steps::apply(index, step, &context).await?);
    }
    Ok(Output { steps: observed })
}

// B304's counters over the local store the windows read: the probe has no shared store to flush to
fn open_router(
    clock: &Arc<AtomicU64>,
) -> Result<CostRouter<WindowCounters<NoSharedStore>>, StoreError> {
    let values = Arc::new(RoutingValueStore::new(
        local_settings(),
        value_clock(clock),
    )?);
    let counters = Arc::new(RouterCounters::new(
        router_settings(),
        Arc::clone(&values),
        None::<Arc<NoSharedStore>>,
    ));
    Ok(CostRouter::new(
        WindowCounters::new(counters, values),
        io_clock(clock),
    ))
}

// one deployment dict of the pinned model_list, its prices and limits in the sources the selector reads
fn build_deployment(group: &str, input: &DeploymentInput) -> Result<DeploymentSpec, ScenarioError> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(input.model.clone()));
    insert(
        &mut params,
        "input_cost_per_token",
        input.input_cost_per_token.as_ref(),
    );
    insert(
        &mut params,
        "output_cost_per_token",
        input.output_cost_per_token.as_ref(),
    );
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
    if let Some(value) = value {
        map.insert(field.to_owned(), value.clone());
    }
}

pub(super) fn now(clock: &Arc<AtomicU64>) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(clock.load(Ordering::SeqCst))
}

fn io_clock(clock: &Arc<AtomicU64>) -> IoClock {
    let source = Arc::clone(clock);
    let read = move || UNIX_EPOCH + Duration::from_millis(source.load(Ordering::SeqCst));
    Arc::new(read)
}

fn value_clock(clock: &Arc<AtomicU64>) -> Clock {
    let source = Arc::clone(clock);
    let read = move || Duration::from_millis(source.load(Ordering::SeqCst)).as_secs_f64();
    Arc::new(read)
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: Some(1024),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    }
}

const fn router_settings() -> Settings {
    Settings {
        sync_interval: None,
        default_sync_interval: Duration::from_secs(1),
        should_batch_redis_writes: false,
    }
}
