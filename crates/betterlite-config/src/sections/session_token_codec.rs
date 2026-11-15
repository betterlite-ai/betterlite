use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number};
use crate::sections::Section;

mod tests;

const BUDGET_PATH: &str = "litellm_settings.max_ui_session_budget";
const DEFAULT_CLI_HOURS: i64 = 24;
const CLI_HOURS_ENVIRONMENTS: [&str; 2] = [
    "CLI_JWT_EXPIRATION_HOURS",
    "LITELLM_CLI_JWT_EXPIRATION_HOURS",
];

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    #[serde(default)]
    max_ui_session_budget: RawBudget,
}

#[derive(Debug, Default, Deserialize)]
#[serde(from = "Option<RawNumber>")]
struct RawBudget(BudgetState);

// a present null is a disabled budget, an absent key keeps LiteLLM's one-dollar default
#[derive(Debug, Default)]
enum BudgetState {
    #[default]
    Missing,
    Null,
    Number(RawNumber),
}

impl From<Option<RawNumber>> for RawBudget {
    fn from(number: Option<RawNumber>) -> Self {
        Self(number.map_or(BudgetState::Null, BudgetState::Number))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub max_ui_session_budget: Option<Decimal>,
    pub cli_expiration_hours: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_ui_session_budget: Some(Decimal::ONE),
            cli_expiration_hours: DEFAULT_CLI_HOURS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let max_ui_session_budget = match self.litellm_settings.max_ui_session_budget.0 {
            BudgetState::Missing => Some(Decimal::ONE),
            BudgetState::Null => None,
            BudgetState::Number(number) => {
                resolve_number(Some(number), BUDGET_PATH, lookup, parse_money)?
            }
        };
        Ok(Settings {
            max_ui_session_budget,
            cli_expiration_hours: cli_expiration_hours(lookup)?,
        })
    }
}

fn cli_expiration_hours<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    for name in CLI_HOURS_ENVIRONMENTS {
        if let Some(value) = lookup.lookup(name)
            && let Some(hours) = betterlite_env::number::<i64>(name, &value)?
        {
            return Ok(hours);
        }
    }
    Ok(DEFAULT_CLI_HOURS)
}
