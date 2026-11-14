use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::budget_windows as config;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_router::budget_increment::dual::DualWindowCache;
use betterlite_router::budget_increment::redis_cache_window::RedisCacheWindow;
use betterlite_router::budget_windows::service::{BudgetWindows, should_init_budget_limiter};
use betterlite_router::budget_windows::window::init_provider_budget_in_cache;
use betterlite_server::routing::budget_windows::{PeriodParser, budget_windows};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod steps;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub provider_budget_config: Option<BTreeMap<String, Provider>>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Build,
    Init,
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
    pub ttl_seconds: Option<u64>,
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

type ProbeCache = DualWindowCache<RedisCacheWindow>;

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
    let database = TimeZoneDatabase::from_env();
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
    let cache = started_cache(&windows).await?;
    let mut outputs = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outputs.push(steps::run_step(step, &windows, &cache, initialized).await?);
    }
    Ok(Output { steps: outputs })
}

// _init_provider_budgets starts every provider window when the limiter is built
async fn started_cache(windows: &BudgetWindows) -> Result<Arc<ProbeCache>, String> {
    let memory = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(|error| error.to_string())?;
    let cache = Arc::new(DualWindowCache {
        memory: Arc::new(Mutex::new(memory)),
        redis: None,
    });
    for (provider, budget) in windows.provider.iter().flatten() {
        init_provider_budget_in_cache(cache.as_ref(), provider, budget, 0.0).await;
    }
    Ok(cache)
}

fn section(input: &Input) -> Result<config::Settings, String> {
    let mut deployments = Vec::with_capacity(input.deployments.len());
    for deployment in &input.deployments {
        deployments.push(config::DeploymentBudgetConfig {
            max_budget: money(deployment.max_budget.as_deref())?,
            budget_duration: deployment.budget_duration.clone(),
        });
    }
    let provider_budget_config = input
        .provider_budget_config
        .as_ref()
        .map(|providers| {
            providers
                .iter()
                .map(|(name, provider)| -> Result<_, String> {
                    let limit = money(provider.budget_limit.as_deref())?;
                    let duration = provider.time_period.clone();
                    Ok((name.clone(), config::BudgetValues { limit, duration }))
                })
                .collect::<Result<IndexMap<_, _>, String>>()
        })
        .transpose()?;
    let tag_budget_config = input
        .tag_budget_config
        .as_ref()
        .map(|tags| {
            tags.iter()
                .map(|(name, tag)| -> Result<_, String> {
                    let limit = money(tag.max_budget.as_deref())?;
                    let duration = tag.budget_duration.clone();
                    Ok((name.clone(), config::BudgetValues { limit, duration }))
                })
                .collect::<Result<IndexMap<_, _>, String>>()
        })
        .transpose()?;
    Ok(config::Settings {
        provider_budget_config,
        tag_budget_config,
        deployments,
    })
}

fn money(value: Option<&str>) -> Result<Option<Decimal>, String> {
    value
        .map(|text| Decimal::from_str(text).map_err(|error| error.to_string()))
        .transpose()
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
