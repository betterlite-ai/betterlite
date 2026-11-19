use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PROVIDER_BUDGET_PATH: &str = "router_settings.provider_budget_config";
const ENV_PREFIX: &str = "os.environ/";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawRouter {
    provider_budget_config: Option<IndexMap<String, Option<RawBudget>>>,
}

// _init_provider_budgets reads exactly these two keys from a provider entry, so BudgetConfig's other names never count
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawBudget {
    budget_limit: Option<RawNumber>,
    time_period: Option<String>,
}

// One provider entry as BudgetConfig keeps it: budget_limit becomes max_budget and time_period becomes budget_duration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderBudgetConfig {
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub provider_budget_config: Option<IndexMap<String, ProviderBudgetConfig>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(entries) = self.router_settings.provider_budget_config else {
            return Ok(Settings::default());
        };
        let mut resolved = IndexMap::with_capacity(entries.len());
        for (provider, entry) in entries {
            let path = format!("{PROVIDER_BUDGET_PATH}.{provider}");
            // _init_provider_budgets raises for a provider whose config is null, so the load fails here
            let Some(budget) = entry else {
                return Err(ConfigError::InvalidScalar { path, value: false });
            };
            let max_budget = resolve_number(
                budget.budget_limit,
                &format!("{path}.budget_limit"),
                lookup,
                parse_money,
            )?;
            let budget_duration =
                duration(budget.time_period, &format!("{path}.time_period"), lookup)?;
            resolved.insert(
                provider,
                ProviderBudgetConfig {
                    max_budget,
                    budget_duration,
                },
            );
        }
        Ok(Settings {
            provider_budget_config: Some(resolved),
        })
    }
}

pub fn load_provider_budget_view<L>(source: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    Section::convert(raw, lookup)
}

// the duration text is never interpreted here, and only an environment reference that resolves empty counts as unset
fn duration<L>(value: Option<String>, path: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let from_environment = text.starts_with(ENV_PREFIX);
    let resolved = resolve_string(text, path, lookup)?;
    Ok(Some(resolved).filter(|candidate| !from_environment || !candidate.is_empty()))
}
