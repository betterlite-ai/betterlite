pub mod error;
mod tests;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_with::rust::double_option;

use crate::budget_primitives::window_start::DurationParser;

use super::amount_text::amount_from_text;
use error::BudgetConfigError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetConfig {
    pub max_budget: Option<Decimal>,
    pub budget_duration: String,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
}

#[derive(Clone, Copy)]
pub struct BudgetRuntime<'parser> {
    pub durations: &'parser (dyn DurationParser + Sync),
    pub now: DateTime<Utc>,
}

impl BudgetRuntime<'_> {
    pub fn budget_config(&self, raw: &JsonData) -> Result<BudgetConfig, BudgetConfigError> {
        let entry = parse_entry(raw)?;
        // BudgetConfig.__init__ renames time_period and budget_limit over the long names, null included
        let duration = match entry.time_period {
            Some(period) => period,
            None => entry.budget_duration,
        };
        let Some(budget_duration) = duration else {
            return Err(BudgetConfigError::MissingDuration);
        };
        if self.durations.seconds(&budget_duration, self.now).is_none() {
            return Err(BudgetConfigError::UnparsableDuration {
                spec: budget_duration,
            });
        }
        let limit = match entry.budget_limit {
            Some(limit) => limit,
            None => entry.max_budget,
        };
        let max_budget = match limit {
            Some(number) => {
                Some(amount_from_text(&number.to_string()).ok_or(BudgetConfigError::MaxBudget)?)
            }
            None => None,
        };
        Ok(BudgetConfig {
            max_budget,
            budget_duration,
            tpm_limit: entry.tpm_limit,
            rpm_limit: entry.rpm_limit,
        })
    }
}

#[derive(Deserialize)]
#[expect(
    clippy::option_option,
    reason = "the outer Option records that the key was present, so a null still overrides the long name"
)]
struct RawBudgetConfig {
    #[serde(default, with = "double_option")]
    time_period: Option<Option<String>>,
    #[serde(default)]
    budget_duration: Option<String>,
    #[serde(default, with = "double_option")]
    budget_limit: Option<Option<serde_json::Number>>,
    #[serde(default)]
    max_budget: Option<serde_json::Number>,
    #[serde(default)]
    tpm_limit: Option<i64>,
    #[serde(default)]
    rpm_limit: Option<i64>,
}

fn parse_entry(raw: &JsonData) -> Result<RawBudgetConfig, BudgetConfigError> {
    // serde reads a JSON list as struct field values, so only an object may reach the struct
    let JsonData::Object(_) = raw else {
        return Err(BudgetConfigError::NotObject);
    };
    serde_json::to_string(raw)
        .and_then(|text| serde_json::from_str::<RawBudgetConfig>(&text))
        .map_err(|source| BudgetConfigError::Entry { source })
}
