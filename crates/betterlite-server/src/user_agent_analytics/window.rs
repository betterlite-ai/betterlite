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
    let bucket = match report {
        ActiveUsersReport::Daily => {
            return Ok(TagActiveUsersResponse {
                tag: row.tag,
                active_users: row.active_users,
                date: row.date,
                period_start: None,
                period_end: None,
            });
        }
        ActiveUsersReport::Weekly => WEEK_BUCKET,
        ActiveUsersReport::Monthly => MONTH_BUCKET,
    };
    bucketed_result(window, row, &bucket)
}

struct Bucket {
    noun: &'static str,
    limit: u64,
    last_offset: u64,
    days: u64,
    period_length: u64,
    label_format: &'static str,
}

// weekly keeps the Week 0 label a checked subtraction gives offset seven, where monthly rejects that offset
const WEEK_BUCKET: Bucket = Bucket {
    noun: "Week",
    limit: MAX_WEEKS,
    last_offset: MAX_WEEKS,
    days: WEEK_DAYS,
    period_length: PERIOD_START_DAYS,
    label_format: LABEL_DATE_FORMAT,
};

const MONTH_BUCKET: Bucket = Bucket {
    noun: "Month",
    limit: MAX_MONTHS,
    last_offset: MAX_MONTHS - 1,
    days: MONTH_DAYS,
    period_length: PERIOD_LAST_DAYS,
    label_format: MONTH_LABEL_FORMAT,
};

fn bucketed_result(
    window: &Window,
    row: ActiveUsersRow,
    bucket: &Bucket,
) -> Result<TagActiveUsersResponse, AnalyticsError> {
    let ActiveUsersRow {
        tag,
        active_users,
        date,
    } = row;
    let bucket_offset = offset_of(&date)?;
    let number = bucket_number(bucket, bucket_offset)?;
    let (period_start, period_end) =
        period_of(window, bucket_offset, bucket.days, bucket.period_length)?;
    Ok(TagActiveUsersResponse {
        tag,
        active_users,
        date: format!(
            "{} {number} ({})",
            bucket.noun,
            period_start.format(bucket.label_format)
        ),
        period_start: Some(period_start.format(DATE_FORMAT).to_string()),
        period_end: Some(period_end.format(DATE_FORMAT).to_string()),
    })
}

fn bucket_number(bucket: &Bucket, bucket_offset: i64) -> Result<u64, AnalyticsError> {
    let offset = u64::try_from(bucket_offset).map_err(|_negative| AnalyticsError::Offset {
        offset: bucket_offset,
    })?;
    if offset > bucket.last_offset {
        return Err(AnalyticsError::Offset {
            offset: bucket_offset,
        });
    }
    bucket
        .limit
        .checked_sub(offset)
        .ok_or(AnalyticsError::Offset {
            offset: bucket_offset,
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
