use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    provider_budget_config: Option<IndexMap<String, RawBudget>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    tag_budget_config: Option<IndexMap<String, RawBudget>>,
}

// every field LiteLLM's BudgetConfig declares, since the tag tier passes the dict through that model
#[derive(Debug, Default, Deserialize)]
struct RawBudget {
    budget_limit: Option<RawNumber>,
    max_budget: Option<RawNumber>,
    time_period: Option<String>,
    budget_duration: Option<String>,
    #[serde(rename = "tpm_limit")]
    _tpm_limit: Option<RawNumber>,
    #[serde(rename = "rpm_limit")]
    _rpm_limit: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    max_budget: Option<RawNumber>,
    budget_duration: Option<String>,
}

// one resolved budget config: LiteLLM keeps the limit as max_budget and the window as budget_duration
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetValues {
    pub limit: Option<Decimal>,
    pub duration: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentBudgetConfig {
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<String>,
}

// an absent provider_budget_config leaves provider limiting off, while an empty map still counts as configured
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub provider_budget_config: Option<IndexMap<String, BudgetValues>>,
    pub tag_budget_config: Option<IndexMap<String, BudgetValues>>,
    pub deployments: Vec<DeploymentBudgetConfig>,
}

#[derive(Clone, Copy)]
enum Tier {
    Provider,
    Tag,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            provider_budget_config: tier_config(
                self.router_settings.provider_budget_config,
                "router_settings.provider_budget_config",
                Tier::Provider,
                lookup,
            )?,
            tag_budget_config: tier_config(
                self.litellm_settings.tag_budget_config,
                "litellm_settings.tag_budget_config",
                Tier::Tag,
                lookup,
            )?,
            deployments: deployments(self.model_list, lookup)?,
        })
    }
}

// the provider tier reads only budget_limit and time_period, while the tag tier also accepts BudgetConfig's aliases
fn tier_config<L>(
    raw: Option<IndexMap<String, RawBudget>>,
    prefix: &str,
    tier: Tier,
    lookup: &L,
) -> Result<Option<IndexMap<String, BudgetValues>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut resolved = IndexMap::with_capacity(raw.len());
    for (name, entry) in raw {
        let path = format!("{prefix}.{name}");
        let (limit, duration) = match tier {
            Tier::Provider => (
                resolve_number(
                    entry.budget_limit,
                    &format!("{path}.budget_limit"),
                    lookup,
                    parse_money,
                )?,
                period(entry.time_period, &format!("{path}.time_period"), lookup)?,
            ),
            // BudgetConfig.__init__ moves budget_limit onto max_budget and time_period onto budget_duration
            Tier::Tag => (
                resolve_number(
                    entry.budget_limit.or(entry.max_budget),
                    &format!("{path}.max_budget"),
                    lookup,
                    parse_money,
                )?,
                period(
                    entry.time_period.or(entry.budget_duration),
                    &format!("{path}.budget_duration"),
                    lookup,
                )?,
            ),
        };
        resolved.insert(name, BudgetValues { limit, duration });
    }
    Ok(Some(resolved))
}

fn deployments<L>(
    raw: Vec<RawDeployment>,
    lookup: &L,
) -> Result<Vec<DeploymentBudgetConfig>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut deployments = Vec::with_capacity(raw.len());
    for (index, deployment) in raw.into_iter().enumerate() {
        let prefix = format!("model_list.{index}.litellm_params");
        deployments.push(DeploymentBudgetConfig {
            max_budget: resolve_number(
                deployment.litellm_params.max_budget,
                &format!("{prefix}.max_budget"),
                lookup,
                parse_money,
            )?,
            budget_duration: period(
                deployment.litellm_params.budget_duration,
                &format!("{prefix}.budget_duration"),
                lookup,
            )?,
        });
    }
    Ok(deployments)
}

fn parse_money(text: &str, path: &str) -> Result<Decimal, ConfigError> {
    text.parse::<Decimal>()
        .map_err(|_source| ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        })
}

// a resolved empty environment string counts as unset, as every other config string does
fn period<L>(value: Option<String>, path: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()?;
    Ok(resolved.filter(|text| !text.is_empty()))
}
