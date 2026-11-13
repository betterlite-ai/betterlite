use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::time::Duration;

use betterlite_config::sections::budget_windows as config;
use betterlite_router::budget_windows::keys::WindowKeys;
use betterlite_router::budget_windows::service::{BudgetWindows, should_init_budget_limiter};
use betterlite_router::budget_windows::test_support::MemoryStore;
use betterlite_router::budget_windows::window::{
    NewWindow, get_or_set_budget_start_time, handle_new_budget_window,
};
use betterlite_server::routing::budget_windows::{PeriodParser, budget_windows};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub tag_budget_config: Option<BTreeMap<String, Tag>>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub model_id: String,
    #[serde(default)]
    pub max_budget: Option<String>,
    #[serde(default)]
    pub budget_duration: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    #[serde(default)]
    pub max_budget: Option<String>,
    #[serde(default)]
    pub budget_duration: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Build,
    Start,
    Reset,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub start_time_key: Option<String>,
    #[serde(default)]
    pub spend_key: Option<String>,
    #[serde(default)]
    pub current_time: Option<String>,
    #[serde(default)]
    pub response_cost: Option<String>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub kind: StepKind,
    pub should_init: Option<bool>,
    pub limiter: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub window_started: Option<bool>,
    pub spend: Option<String>,
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
        .block_on(run_steps(input))
        .map_err(|message| invalid(&message))?;
    Ok(ProbeOutput::BudgetWindows(output))
}

async fn run_steps(input: &Input) -> Result<Output, String> {
    let section = section(input)?;
    let model_ids: Vec<Option<&str>> = input
        .deployments
        .iter()
        .map(|deployment| Some(deployment.model_id.as_str()))
        .collect();
    let now = DateTime::<Utc>::from_timestamp(0, 0).ok_or("the probe clock is out of range")?;
    let database = TimeZoneDatabase::default();
    let parser = PeriodParser {
        now,
        timezone: None,
        database: &database,
    };
    let resolved =
        budget_windows(&section, &model_ids, parser).map_err(|error| error.to_string())?;
    let initialized = should_init_budget_limiter(
        resolved.provider_budget_config.as_ref(),
        resolved.tag_budget_config.as_ref(),
        Some(&resolved.deployments),
    );
    let windows = BudgetWindows::new(
        resolved.provider_budget_config,
        resolved.tag_budget_config,
        &resolved.deployments,
    );
    let store = MemoryStore::default();
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let output = match step.kind {
            StepKind::Build => build_step(&windows, initialized),
            StepKind::Start => start_step(step, &windows, &store).await?,
            StepKind::Reset => reset_step(step, &store).await?,
        };
        steps.push(output);
    }
    Ok(Output { steps })
}

fn section(input: &Input) -> Result<config::Settings, String> {
    let mut deployments = Vec::with_capacity(input.deployments.len());
    for deployment in &input.deployments {
        deployments.push(config::DeploymentBudgetConfig {
            max_budget: money(&deployment.max_budget)?,
            budget_duration: deployment.budget_duration.clone(),
        });
    }
    let tag_budget_config = match &input.tag_budget_config {
        None => None,
        Some(tags) => {
            let mut resolved = indexmap::IndexMap::with_capacity(tags.len());
            for (name, tag) in tags {
                resolved.insert(
                    name.clone(),
                    config::BudgetValues {
                        limit: money(&tag.max_budget)?,
                        duration: tag.budget_duration.clone(),
                    },
                );
            }
            Some(resolved)
        }
    };
    Ok(config::Settings {
        provider_budget_config: None,
        tag_budget_config,
        deployments,
    })
}

fn money(value: &Option<String>) -> Result<Option<Decimal>, String> {
    value
        .as_deref()
        .map(|text| Decimal::from_str(text).map_err(|error| error.to_string()))
        .transpose()
}

fn build_step(windows: &BudgetWindows, initialized: bool) -> StepOutput {
    StepOutput {
        kind: StepKind::Build,
        should_init: Some(initialized),
        limiter: Some(initialized),
        tags: windows
            .tag
            .as_ref()
            .map(|configs| configs.keys().cloned().collect()),
        window_started: None,
        spend: None,
    }
}

// LiteLLM's success event starts the window with the deployment's own duration as the ttl
async fn start_step(
    step: &Step,
    windows: &BudgetWindows,
    store: &MemoryStore,
) -> Result<StepOutput, String> {
    let model_id = step
        .model_id
        .as_deref()
        .ok_or("the start step needs a model_id")?;
    let start_time_key = step
        .start_time_key
        .as_deref()
        .ok_or("the start step needs a start_time_key")?;
    let current_time = current_time(step)?;
    if let Some(budget) = windows.deployment_budget(model_id) {
        get_or_set_budget_start_time(
            store,
            start_time_key,
            current_time,
            Some(budget.period.seconds),
        )
        .await;
    }
    Ok(StepOutput {
        kind: StepKind::Start,
        should_init: None,
        limiter: None,
        tags: None,
        window_started: Some(store.observe(start_time_key, start_time_key).started),
        spend: None,
    })
}

async fn reset_step(step: &Step, store: &MemoryStore) -> Result<StepOutput, String> {
    let spend_key = step
        .spend_key
        .as_deref()
        .ok_or("the reset step needs a spend_key")?;
    let start_time_key = step
        .start_time_key
        .as_deref()
        .ok_or("the reset step needs a start_time_key")?;
    let current_time = current_time(step)?;
    let ttl_seconds = step.ttl_seconds.ok_or("the reset step needs ttl_seconds")?;
    let ttl = duration(ttl_seconds)?;
    let cost = Decimal::from_str(
        step.response_cost
            .as_deref()
            .ok_or("the reset step needs response_cost")?,
    )
    .map_err(|error| error.to_string())?;
    store.seed_start(start_time_key, 0.0, ttl);
    let keys = WindowKeys {
        spend: spend_key.to_owned(),
        start_time: start_time_key.to_owned(),
    };
    handle_new_budget_window(
        store,
        NewWindow {
            keys: &keys,
            current_time,
            response_cost: cost,
            ttl: Some(ttl),
        },
    )
    .await;
    let observed = store.observe(start_time_key, spend_key);
    Ok(StepOutput {
        kind: StepKind::Reset,
        should_init: None,
        limiter: None,
        tags: None,
        window_started: Some(observed.started),
        spend: observed.spend.map(|spend| spend.normalize().to_string()),
    })
}

fn current_time(step: &Step) -> Result<f64, String> {
    f64::from_str(
        step.current_time
            .as_deref()
            .ok_or("the step needs current_time")?,
    )
    .map_err(|error| error.to_string())
}

fn duration(seconds: i64) -> Result<Duration, String> {
    u64::try_from(seconds)
        .map(Duration::from_secs)
        .map_err(|_source| format!("the ttl of {seconds} seconds is negative"))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
