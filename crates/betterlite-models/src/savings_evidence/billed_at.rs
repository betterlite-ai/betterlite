use chrono::{DateTime, Utc};

mod iso;

pub enum BilledAt {
    Missing,
    Timestamp(DateTime<Utc>),
    Text(String),
}

// Mirrors _coerce_billed_at's fromisoformat(value.replace("Z", "+00:00")), None instead of ValueError.
#[must_use]
pub fn decode_billed_at(text: &str) -> Option<DateTime<Utc>> {
    let normalized = text.replace('Z', "+00:00");
    let chars: Vec<char> = normalized.chars().collect();
    iso::parse(&chars)
}

#[must_use]
pub fn coerce_billed_at(value: BilledAt) -> Option<DateTime<Utc>> {
    match value {
        BilledAt::Missing => None,
        BilledAt::Timestamp(timestamp) => Some(timestamp),
        BilledAt::Text(text) => decode_billed_at(&text),
    }
}

mod tests;
