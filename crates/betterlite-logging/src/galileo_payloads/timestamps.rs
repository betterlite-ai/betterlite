use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, Utc};
use regex::Regex;

// A datetime as LiteLLM's callers pass it: with or without a tzinfo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawTimestamp {
    Naive(NaiveDateTime),
    Aware(DateTime<FixedOffset>),
}

// litellm.integrations.galileo.GalileoObserve._format_created_at/._local_timezone; a naive datetime is assumed local (the caller's injected offset) and converted to UTC, an aware one converts directly.
#[must_use]
pub fn format_created_at(timestamp: RawTimestamp, local_offset: FixedOffset) -> String {
    let utc = match timestamp {
        RawTimestamp::Naive(naive) => {
            let shift = Duration::seconds(i64::from(local_offset.local_minus_utc()));
            let shifted = naive.checked_sub_signed(shift).unwrap_or(naive);
            DateTime::<Utc>::from_naive_utc_and_offset(shifted, Utc)
        }
        RawTimestamp::Aware(aware) => aware.with_timezone(&Utc),
    };
    utc.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// litellm.integrations.galileo.GalileoObserve._normalize_created_at.
#[must_use]
pub fn normalize_created_at(created_at: &str) -> String {
    let Ok(timezone_suffix) = Regex::new(r"(Z|[+-]\d{2}:?\d{2})$") else {
        return created_at.to_owned();
    };
    if !created_at.is_empty() && !timezone_suffix.is_match(created_at) {
        format!("{created_at}Z")
    } else {
        created_at.to_owned()
    }
}
