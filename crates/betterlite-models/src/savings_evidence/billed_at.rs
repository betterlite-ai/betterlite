use chrono::{DateTime, Utc};

pub enum BilledAt {
    Missing,
    Timestamp(DateTime<Utc>),
    Text(String),
}

// Mirrors datetime.fromisoformat(value.replace("Z", "+00:00")), requiring an explicit offset.
#[must_use]
pub fn decode_billed_at(text: &str) -> Option<DateTime<Utc>> {
    let normalized = text.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
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
