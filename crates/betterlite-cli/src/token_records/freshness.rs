use std::time::Duration;

use super::record::CliTokenRecord;

pub const TOKEN_FRESHNESS_BUFFER_SECONDS: u64 = 360;

const SECONDS_PER_HOUR: i128 = 3_600;
const NANOS_PER_SECOND: i128 = 1_000_000_000;

#[must_use]
pub fn is_token_fresh(record: &CliTokenRecord, now_seconds: f64, expiration_hours: i64) -> bool {
    let Some(now) = Duration::try_from_secs_f64(now_seconds).ok() else {
        return false;
    };
    if let Some(expires_at) = record.expires_at {
        let Ok(deadline) = Duration::try_from_secs_f64(expires_at) else {
            return false;
        };
        return deadline.saturating_sub(now) > Duration::from_secs(TOKEN_FRESHNESS_BUFFER_SECONDS);
    }
    let Some(stamp) = Duration::try_from_secs_f64(record.timestamp).ok() else {
        return false;
    };
    let (Ok(now_nanos), Ok(stamp_nanos)) = (
        i128::try_from(now.as_nanos()),
        i128::try_from(stamp.as_nanos()),
    ) else {
        return false;
    };
    let window = i128::from(expiration_hours)
        .checked_mul(SECONDS_PER_HOUR)
        .and_then(|seconds| seconds.checked_sub(i128::from(TOKEN_FRESHNESS_BUFFER_SECONDS)))
        .and_then(|seconds| seconds.checked_mul(NANOS_PER_SECOND));
    window
        .and_then(|span| stamp_nanos.checked_add(span))
        .is_some_and(|deadline| now_nanos < deadline)
}
