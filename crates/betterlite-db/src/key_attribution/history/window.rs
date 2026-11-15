use betterlite_models::key_digest::is_valid_sha256_hash;
use chrono::NaiveDateTime;

const HASHED_JWT_PREFIX: &str = "hashed-jwt-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryWindow {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

#[must_use]
pub fn is_spend_log_digest(key: &str) -> bool {
    is_valid_sha256_hash(key.strip_prefix(HASHED_JWT_PREFIX).unwrap_or(key))
}

#[must_use]
pub fn isoformat(value: NaiveDateTime) -> String {
    if value.and_utc().timestamp_subsec_micros() == 0 {
        return value.format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    value.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

#[must_use]
pub fn spend_log_cache_key(digest: &str, window: &HistoryWindow) -> String {
    let start = isoformat(window.start);
    let end = isoformat(window.end);
    format!("spend_log_key_metadata:{digest}:{start}:{end}")
}
