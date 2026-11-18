use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::budget_windows as config;
use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use betterlite_router::budget_increment::dual::DualWindowCache;
use betterlite_router::budget_increment::increment::BudgetIncrement;
use betterlite_router::budget_windows::service::BudgetWindows;
use betterlite_router::routing_budget_state::error::BudgetStateError;
use betterlite_router::routing_budget_state::service::RoutingBudgetState;
use betterlite_router::routing_value_store::Clock;
use betterlite_server::routing::budget_windows::{
    BudgetWindowsError, PeriodParser, budget_windows,
};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};
use tokio_util::task::TaskTracker;

use super::steps;
use super::store::ProbeStore;

const MEMORY_CAPACITY: i64 = 200;
const MEMORY_DEFAULT_TTL_SECONDS: f64 = 600.0;
const MEMORY_ITEM_KIB: i64 = 1024;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    #[serde(default)]
    pub provider_budget_config: Option<BTreeMap<String, Provider>>,
    #[serde(default)]
    pub tag_budget_config: Option<BTreeMap<String, Tag>>,
    #[serde(default)]
    pub deployments: Vec<Deployment>,
    pub steps: Vec<ScenarioStep>,
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
pub struct Provider {
    #[serde(default)]
    pub budget_limit: Option<String>,
    #[serde(default)]
    pub time_period: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    #[serde(default)]
    pub max_budget: Option<String>,
    #[serde(default)]
    pub budget_duration: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub kind: StepKind,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub cost: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Success,
    Sync,
    BeginSync,
    EndSync,
    Shutdown,
    Memory,
    Redis,
    FailWrites,
    SucceedWrites,
    HoldWrites,
    ReleaseWrites,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioOutput {
    pub steps: Vec<Observation>,
}

// one entry per step: the value the step made observable, tagged so every answer keeps its type
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub kind: StepKind,
    pub step: usize,
    pub value: Option<String>,
}

// the adapter's own failures, each holding the error it came from until the probe boundary renders it
#[derive(Debug, Error)]
pub(super) enum ScenarioError {
    #[error("the budget windows could not be resolved")]
    Windows {
        #[from]
        source: BudgetWindowsError,
    },
    #[error("the local budget window cache failed")]
    Local {
        #[from]
        source: CacheError,
    },
    #[error("a logged success failed to spend")]
    State {
        #[from]
        source: BudgetStateError,
    },
    #[error("a spawned sync failed")]
    Join {
        #[source]
        source: JoinError,
    },
    #[error("the step needs {what}")]
    Missing { what: &'static str },
    #[error("{text:?} is not money")]
    Money {
        text: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the probe clock is out of range")]
    Clock,
    #[error("begin_sync needs hold_writes before it")]
    NotHeld,
    #[error("end_sync needs a begin_sync before it")]
    BeyondSync,
}

pub(super) struct Run {
    pub(super) state: Arc<RoutingBudgetState<ProbeStore>>,
    pub(super) memory: Arc<Mutex<LocalTtlCache>>,
    pub(super) store: ProbeStore,
    pub(super) syncs: JoinSet<bool>,
}

pub(super) async fn run_scenario(input: &ScenarioInput) -> Result<ScenarioOutput, ScenarioError> {
    let windows = windows(input)?;
    let clock: Clock = Arc::new(|| 0.0_f64);
    let memory = LocalTtlCache::with_clock(
        Some(MEMORY_CAPACITY),
        Some(MEMORY_DEFAULT_TTL_SECONDS),
        None,
        MEMORY_ITEM_KIB,
        Arc::clone(&clock),
    )?;
    let store = ProbeStore::default();
    let cache = Arc::new(DualWindowCache {
        memory: Arc::new(Mutex::new(memory)),
        redis: Some(store.clone()),
    });
    let memory = Arc::clone(&cache.memory);
    let state = Arc::new(RoutingBudgetState {
        increments: BudgetIncrement {
            cache,
            clock,
            queue: Arc::default(),
            flush_lock: Arc::default(),
        },
        windows,
        sync: Mutex::new(JoinSet::new()),
        flushes: TaskTracker::new(),
    });
    let mut run = Run {
        state,
        memory,
        store,
        syncs: JoinSet::new(),
    };
    let mut observations = Vec::with_capacity(input.steps.len());
    for (index, step) in input.steps.iter().enumerate() {
        observations.push(steps::apply(index, step, &mut run).await?);
    }
    Ok(ScenarioOutput {
        steps: observations,
    })
}

fn windows(input: &ScenarioInput) -> Result<BudgetWindows, ScenarioError> {
    let section = section(input)?;
    let model_ids: Vec<Option<&str>> = input
        .deployments
        .iter()
        .map(|deployment| Some(deployment.model_id.as_str()))
        .collect();
    let now = DateTime::<Utc>::from_timestamp(0, 0).ok_or(ScenarioError::Clock)?;
    let database = TimeZoneDatabase::from_env();
    let parser = PeriodParser {
        now,
        timezone: None,
        database: &database,
    };
    let resolved = budget_windows(&section, &model_ids, parser)?;
    Ok(BudgetWindows::new(
        resolved.provider_budget_config,
        resolved.tag_budget_config,
        &resolved.deployments,
    ))
}

fn section(input: &ScenarioInput) -> Result<config::Settings, ScenarioError> {
    let mut deployments = Vec::with_capacity(input.deployments.len());
    for deployment in &input.deployments {
        deployments.push(config::DeploymentBudgetConfig {
            max_budget: deployment
                .max_budget
                .as_deref()
                .map(money_text)
                .transpose()?,
            budget_duration: deployment.budget_duration.clone(),
        });
    }
    let provider_budget_config = input
        .provider_budget_config
        .as_ref()
        .map(|providers| {
            budget_map(providers.iter().map(|(name, provider)| {
                (
                    name,
                    provider.budget_limit.as_deref(),
                    provider.time_period.as_deref(),
                )
            }))
        })
        .transpose()?;
    let tag_budget_config = input
        .tag_budget_config
        .as_ref()
        .map(|tags| {
            budget_map(tags.iter().map(|(name, tag)| {
                (
                    name,
                    tag.max_budget.as_deref(),
                    tag.budget_duration.as_deref(),
                )
            }))
        })
        .transpose()?;
    Ok(config::Settings {
        provider_budget_config,
        tag_budget_config,
        deployments,
    })
}

fn budget_map<'entry, Entries>(
    entries: Entries,
) -> Result<IndexMap<String, config::BudgetValues>, ScenarioError>
where
    Entries: Iterator<Item = (&'entry String, Option<&'entry str>, Option<&'entry str>)>,
{
    let mut budgets = IndexMap::new();
    for (name, limit, duration) in entries {
        budgets.insert(
            name.clone(),
            config::BudgetValues {
                limit: limit.map(money_text).transpose()?,
                duration: duration.map(str::to_owned),
            },
        );
    }
    Ok(budgets)
}

pub(super) fn money_text(text: &str) -> Result<Decimal, ScenarioError> {
    Decimal::from_str(text).map_err(|source| ScenarioError::Money {
        text: text.to_owned(),
        source,
    })
}
