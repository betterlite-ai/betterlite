use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use jiff::tz::TimeZoneDatabase;

use crate::management_guards::amounts::{Amount, describe_amount};
use crate::management_guards::duration::validate_budget_duration;
use crate::management_guards::error::ManagementGuardError;

// str.strip()'s whitespace set also covers U+001C-U+001F; Rust's is_whitespace() does not
const fn is_strip_whitespace(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        _ => character.is_whitespace(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBudgetConfig<'duration> {
    pub max_budget: Option<Amount>,
    pub budget_duration: Option<&'duration str>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
}

pub fn validate_team_model_max_budget(
    model_max_budget: Option<&BTreeMap<String, ModelBudgetConfig<'_>>>,
    settings: &BudgetResetSettings,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<(), ManagementGuardError> {
    let Some(configs) = model_max_budget else {
        return Ok(());
    };
    for (model, config) in configs {
        if model.trim_matches(is_strip_whitespace).is_empty() {
            return Err(ManagementGuardError::EmptyModelName);
        }
        let accepted =
            matches!(config.max_budget, Some(Amount::Finite(value)) if !value.is_sign_negative());
        if !accepted {
            return Err(ManagementGuardError::ModelMaxBudget {
                model: model.clone(),
                received: describe_amount(config.max_budget),
            });
        }
        let Some(duration) = config.budget_duration else {
            return Err(ManagementGuardError::ModelBudgetDuration {
                model: model.clone(),
            });
        };
        validate_budget_duration(
            Some(duration),
            StatusCode::BAD_REQUEST,
            settings,
            clocks,
            database,
        )?;
        if config.tpm_limit.is_some() || config.rpm_limit.is_some() {
            return Err(ManagementGuardError::ModelRateLimit {
                model: model.clone(),
            });
        }
    }
    Ok(())
}
