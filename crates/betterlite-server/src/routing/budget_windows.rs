use std::time::Duration;

use betterlite_config::sections::budget_windows as config;
use betterlite_core::duration::error::DurationError;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_router::budget_windows::budget::{
    BudgetPeriod, DeploymentLimits, ProviderBudget, TagBudget,
};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum BudgetWindowsError {
    #[error("{path} is not a budget duration")]
    Duration {
        path: String,
        #[source]
        source: BudgetPeriodError,
    },
    #[error("model_list.{index} has no deployment in the model catalog")]
    MissingDeployment { index: usize },
}

#[derive(Debug, Error)]
pub enum BudgetPeriodError {
    #[error(transparent)]
    Parse(#[from] DurationError),
    #[error("the budget duration resolves to a negative number of seconds")]
    Negative,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BudgetWindowsSettings {
    pub provider_budget_config: Option<IndexMap<String, ProviderBudget>>,
    pub tag_budget_config: Option<IndexMap<String, TagBudget>>,
    pub deployments: Vec<DeploymentLimits>,
}

// the injected inputs of B199 duration_in_seconds, carried together so every tier parses months the same way
#[derive(Clone, Copy)]
pub struct PeriodParser<'context> {
    pub now: DateTime<Utc>,
    pub timezone: Option<&'context str>,
    pub database: &'context TimeZoneDatabase,
}

impl PeriodParser<'_> {
    fn parse(self, spec: &str) -> Result<BudgetPeriod, BudgetPeriodError> {
        let parsed = duration_in_seconds(spec, self.now, self.timezone, self.database)?;
        let seconds =
            u64::try_from(parsed.seconds).map_err(|_source| BudgetPeriodError::Negative)?;
        for warning in parsed.warnings {
            tracing::warn!(?warning, spec, "the budget duration needs attention");
        }
        Ok(BudgetPeriod {
            spec: spec.to_owned(),
            seconds: Duration::from_secs(seconds),
        })
    }
}

// the strings stay in betterlite-config; the router receives parsed seconds and keeps LiteLLM's configured text for the keys
pub fn budget_windows(
    section: &config::Settings,
    model_ids: &[Option<&str>],
    parser: PeriodParser<'_>,
) -> Result<BudgetWindowsSettings, BudgetWindowsError> {
    Ok(BudgetWindowsSettings {
        provider_budget_config: tier_budgets(
            section.provider_budget_config.as_ref(),
            "router_settings.provider_budget_config",
            parser,
            |budget_limit, period| ProviderBudget {
                budget_limit,
                period,
            },
        )?,
        tag_budget_config: tier_budgets(
            section.tag_budget_config.as_ref(),
            "litellm_settings.tag_budget_config",
            parser,
            |max_budget, period| TagBudget { max_budget, period },
        )?,
        deployments: deployment_limits(&section.deployments, model_ids, parser)?,
    })
}

fn deployment_limits(
    section: &[config::DeploymentBudgetConfig],
    model_ids: &[Option<&str>],
    parser: PeriodParser<'_>,
) -> Result<Vec<DeploymentLimits>, BudgetWindowsError> {
    let mut deployments = Vec::with_capacity(section.len());
    for (index, budget) in section.iter().enumerate() {
        let model_id = model_ids
            .get(index)
            .ok_or(BudgetWindowsError::MissingDeployment { index })?
            .map(str::to_owned);
        let period = optional(
            budget.budget_duration.as_deref(),
            &format!("model_list.{index}.litellm_params.budget_duration"),
            parser,
        )?;
        deployments.push(DeploymentLimits {
            model_id,
            max_budget: budget.max_budget,
            period,
        });
    }
    Ok(deployments)
}

fn tier_budgets<Budget>(
    section: Option<&IndexMap<String, config::BudgetValues>>,
    prefix: &str,
    parser: PeriodParser<'_>,
    build: fn(Option<Decimal>, Option<BudgetPeriod>) -> Budget,
) -> Result<Option<IndexMap<String, Budget>>, BudgetWindowsError> {
    let Some(entries) = section else {
        return Ok(None);
    };
    let mut resolved = IndexMap::with_capacity(entries.len());
    for (name, budget) in entries {
        let period = optional(
            budget.duration.as_deref(),
            &format!("{prefix}.{name}.budget_duration"),
            parser,
        )?;
        resolved.insert(name.clone(), build(budget.limit, period));
    }
    Ok(Some(resolved))
}

fn optional(
    spec: Option<&str>,
    path: &str,
    parser: PeriodParser<'_>,
) -> Result<Option<BudgetPeriod>, BudgetWindowsError> {
    spec.map(|text| parser.parse(text))
        .transpose()
        .map_err(|source| BudgetWindowsError::Duration {
            path: path.to_owned(),
            source,
        })
}
