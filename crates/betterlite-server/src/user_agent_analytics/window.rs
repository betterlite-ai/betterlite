use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::Days;
use chrono::NaiveDate;

use super::error::AnalyticsError;
use super::responses::TagActiveUsersResponse;
use super::rows::ActiveUsersReport;
use super::rows::ActiveUsersRow;

pub(super) const MAX_DAYS: u64 = 7;
pub(super) const MAX_WEEKS: u64 = 7;
pub(super) const WAU_SPAN_DAYS: u64 = 48;
const WEEK_DAYS: u64 = 7;
const PERIOD_START_DAYS: u64 = 6;
const DATE_FORMAT: &str = "%Y-%m-%d";
const LABEL_DATE_FORMAT: &str = "%b %d";

#[derive(Debug)]
pub(super) struct Window {
    pub(super) start_date: String,
    pub(super) end_date: String,
    pub(super) end_day: NaiveDate,
}

impl Window {
    pub(super) fn ending_tomorrow(
        clock: &dyn GatewayRequestClock,
        span_days: u64,
    ) -> Result<Self, AnalyticsError> {
        let end_day = clock
            .utc_now()
            .date_naive()
            .checked_add_days(Days::new(1))
            .ok_or(AnalyticsError::Window)?;
        let start_day = end_day
            .checked_sub_days(Days::new(span_days))
            .ok_or(AnalyticsError::Window)?;
        Ok(Self {
            start_date: start_day.format(DATE_FORMAT).to_string(),
            end_date: end_day.format(DATE_FORMAT).to_string(),
            end_day,
        })
    }
}

pub(super) fn active_users_result(
    window: &Window,
    report: ActiveUsersReport,
    row: ActiveUsersRow,
) -> Result<TagActiveUsersResponse, AnalyticsError> {
    match report {
        ActiveUsersReport::Daily => Ok(TagActiveUsersResponse {
            tag: row.tag,
            active_users: row.active_users,
            date: row.date,
            period_start: None,
            period_end: None,
        }),
        ActiveUsersReport::Weekly => weekly_result(window, row),
    }
}

pub(super) fn weekly_result(
    window: &Window,
    row: ActiveUsersRow,
) -> Result<TagActiveUsersResponse, AnalyticsError> {
    let ActiveUsersRow {
        tag,
        active_users,
        date,
    } = row;
    let Ok(week_offset) = date.parse::<i64>() else {
        return Err(AnalyticsError::WeekBucket { bucket: date });
    };
    let offset = u64::try_from(week_offset)
        .map_err(|_negative| AnalyticsError::WeekOffset { week_offset })?;
    let weeks = offset
        .checked_mul(WEEK_DAYS)
        .ok_or(AnalyticsError::WeekOffset { week_offset })?;
    let number = MAX_WEEKS
        .checked_sub(offset)
        .ok_or(AnalyticsError::WeekOffset { week_offset })?;
    let period_end = window
        .end_day
        .checked_sub_days(Days::new(weeks))
        .ok_or(AnalyticsError::Window)?;
    let period_start = period_end
        .checked_sub_days(Days::new(PERIOD_START_DAYS))
        .ok_or(AnalyticsError::Window)?;
    Ok(TagActiveUsersResponse {
        tag,
        active_users,
        date: format!("Week {number} ({})", period_start.format(LABEL_DATE_FORMAT)),
        period_start: Some(period_start.format(DATE_FORMAT).to_string()),
        period_end: Some(period_end.format(DATE_FORMAT).to_string()),
    })
}
