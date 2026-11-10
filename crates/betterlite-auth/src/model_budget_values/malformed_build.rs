pub mod error;
mod tests;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

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
        let JsonData::Object(entry) = raw else {
            return Err(BudgetConfigError::NotAnObject);
        };
        // BudgetConfig renames time_period and budget_limit over the long names before validating
        let duration_field = entry
            .get("time_period")
            .or_else(|| entry.get("budget_duration"));
        let Some(JsonData::String(budget_duration)) = duration_field else {
            return Err(BudgetConfigError::MissingDuration);
        };
        if self.durations.seconds(budget_duration, self.now).is_none() {
            return Err(BudgetConfigError::UnparsableDuration {
                spec: budget_duration.clone(),
            });
        }
        let max_budget = match entry
            .get("budget_limit")
            .or_else(|| entry.get("max_budget"))
        {
            None | Some(JsonData::Null) => None,
            Some(JsonData::Number(number)) => {
                Some(amount_from_text(&number.to_string()).ok_or(BudgetConfigError::MaxBudget)?)
            }
            Some(_) => return Err(BudgetConfigError::MaxBudget),
        };
        Ok(BudgetConfig {
            max_budget,
            budget_duration: budget_duration.clone(),
            tpm_limit: integer_limit(entry.get("tpm_limit"), "tpm_limit")?,
            rpm_limit: integer_limit(entry.get("rpm_limit"), "rpm_limit")?,
        })
    }
}

fn integer_limit(
    value: Option<&JsonData>,
    field: &'static str,
) -> Result<Option<i64>, BudgetConfigError> {
    match value {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(BudgetConfigError::Limit { field }),
        Some(_) => Err(BudgetConfigError::Limit { field }),
    }
}
