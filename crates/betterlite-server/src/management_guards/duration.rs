use axum::http::StatusCode;
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::{ResetClocks, budget_duration_error};
use jiff::tz::TimeZoneDatabase;

use crate::management_guards::error::ManagementGuardError;

pub fn validate_budget_duration(
    duration: Option<&str>,
    status: StatusCode,
    settings: &BudgetResetSettings,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<(), ManagementGuardError> {
    // B2317 owns duration parsing, the positivity check and the message; this only raises it
    budget_duration_error(duration, settings, clocks, database).map_or(Ok(()), |message| {
        Err(ManagementGuardError::BudgetDuration { message, status })
    })
}
