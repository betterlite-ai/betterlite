use chrono::{DateTime, Utc};

use crate::budget_permission_rows::JsonData;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::search_tools::convert::isoformat;

use super::error::WindowResetError;
use super::json_persist::set_window_reset_at;

// compute_budget_reset_at(budget_duration, settings).isoformat(): the window's next expiry, written into the window before the roll so a failed roll keeps it.
pub(crate) fn advance_window_reset_at<Schedule>(
    row: &str,
    window: &mut JsonData,
    budget_duration: &str,
    schedule: &Schedule,
) -> Result<DateTime<Utc>, WindowResetError<Schedule::Failure>>
where
    Schedule: ResetSchedule,
{
    let next = schedule
        .next_reset_at(budget_duration)
        .map_err(|source| WindowResetError::Schedule { source })?;
    set_window_reset_at(row, window, isoformat(next))?;
    Ok(next)
}

mod tests;
