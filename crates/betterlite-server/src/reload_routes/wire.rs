use chrono::{DateTime, NaiveDate, TimeDelta, Utc};

// Python's datetime.isoformat() on the naive utcnow() value LiteLLM stores for the beta reload times
pub(super) fn naive_utc_isoformat(at: DateTime<Utc>) -> String {
    let naive = at.naive_utc();
    if at.timestamp_subsec_micros() == 0 {
        return naive.format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    naive.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

// Last reload plus the interval while fewer hours than that have passed; timedelta and datetime.max give the same None
pub(super) fn beta_next_run(
    last_reload: DateTime<Utc>,
    interval_hours: i64,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let interval = TimeDelta::try_hours(interval_hours)?;
    if now.signed_duration_since(last_reload) >= interval {
        return None;
    }
    let next_run = last_reload.checked_add_signed(interval)?;
    let latest = NaiveDate::from_ymd_opt(9999, 12, 31)?
        .and_hms_micro_opt(23, 59, 59, 999_999)?
        .and_utc();
    if next_run > latest {
        return None;
    }
    Some(next_run)
}
