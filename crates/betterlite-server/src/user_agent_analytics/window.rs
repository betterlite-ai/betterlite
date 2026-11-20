use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::Days;
use chrono::NaiveDate;

use super::error::AnalyticsError;
use super::responses::TagActiveUsersResponse;
use super::rows::ActiveUsersReport;
use super::rows::ActiveUsersRow;

pub(super) const MAX_DAYS: u64 = 7;
pub(super) const MAX_WEEKS: u64 = 7;
pub(super) const MAX_MONTHS: u64 = 7;
pub(super) const WAU_SPAN_DAYS: u64 = 48;
// seven 30-day months, each including its last day, reach back 209 days from the end boundary
pub(super) const MAU_SPAN_DAYS: u64 = 209;
pub(super) const PER_USER_SPAN_DAYS: u64 = 30;
const WEEK_DAYS: u64 = 7;
const MONTH_DAYS: u64 = 30;
const PERIOD_START_DAYS: u64 = 6;
const PERIOD_LAST_DAYS: u64 = 29;
pub(super) const DATE_FORMAT: &str = "%Y-%m-%d";
const LABEL_DATE_FORMAT: &str = "%b %d";
const MONTH_LABEL_FORMAT: &str = "%b";

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
        ActiveUsersReport::Monthly => monthly_result(window, row),
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
    let week_offset = offset_of(&date)?;
    let offset = u64::try_from(week_offset).map_err(|_negative| AnalyticsError::Offset {
        offset: week_offset,
    })?;
    let week_number = MAX_WEEKS
        .checked_sub(offset)
        .ok_or(AnalyticsError::Offset {
            offset: week_offset,
        })?;
    let (period_start, period_end) = period_of(window, week_offset, WEEK_DAYS, PERIOD_START_DAYS)?;
    Ok(TagActiveUsersResponse {
        tag,
        active_users,
        date: format!(
            "Week {week_number} ({})",
            period_start.format(LABEL_DATE_FORMAT)
        ),
        period_start: Some(period_start.format(DATE_FORMAT).to_string()),
        period_end: Some(period_end.format(DATE_FORMAT).to_string()),
    })
}

pub(super) fn monthly_result(
    window: &Window,
    row: ActiveUsersRow,
) -> Result<TagActiveUsersResponse, AnalyticsError> {
    let ActiveUsersRow {
        tag,
        active_users,
        date,
    } = row;
    let month_offset = offset_of(&date)?;
    let offset = u64::try_from(month_offset).map_err(|_negative| AnalyticsError::Offset {
        offset: month_offset,
    })?;
    if offset >= MAX_MONTHS {
        return Err(AnalyticsError::Offset {
            offset: month_offset,
        });
    }
    let month_number = MAX_MONTHS
        .checked_sub(offset)
        .ok_or(AnalyticsError::Offset {
            offset: month_offset,
        })?;
    let (period_start, period_end) = period_of(window, month_offset, MONTH_DAYS, PERIOD_LAST_DAYS)?;
    Ok(TagActiveUsersResponse {
        tag,
        active_users,
        date: format!(
            "Month {month_number} ({})",
            period_start.format(MONTH_LABEL_FORMAT)
        ),
        period_start: Some(period_start.format(DATE_FORMAT).to_string()),
        period_end: Some(period_end.format(DATE_FORMAT).to_string()),
    })
}

fn offset_of(date: &str) -> Result<i64, AnalyticsError> {
    date.parse::<i64>()
        .map_err(|_error| AnalyticsError::Bucket {
            bucket: date.to_owned(),
        })
}

fn period_of(
    window: &Window,
    bucket_offset: i64,
    days_per_bucket: u64,
    period_length: u64,
) -> Result<(NaiveDate, NaiveDate), AnalyticsError> {
    let offset = u64::try_from(bucket_offset).map_err(|_negative| AnalyticsError::Offset {
        offset: bucket_offset,
    })?;
    let period_days = offset
        .checked_mul(days_per_bucket)
        .ok_or(AnalyticsError::Offset {
            offset: bucket_offset,
        })?;
    let period_end = window
        .end_day
        .checked_sub_days(Days::new(period_days))
        .ok_or(AnalyticsError::Window)?;
    let period_start = period_end
        .checked_sub_days(Days::new(period_length))
        .ok_or(AnalyticsError::Window)?;
    Ok((period_start, period_end))
}
