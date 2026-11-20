use chrono::{DateTime, NaiveDate, NaiveDateTime};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowExpiryError {
    #[error("{text:?} is not an ISO 8601 reset_at timestamp")]
    Unparseable { text: String },
}

const NAIVE_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

// reset_at <= now, compared as naive UTC the way datetime.fromisoformat(...).replace(tzinfo=None) does; no reset_at means the window never expires.
pub(crate) fn window_is_expired(
    reset_at: Option<&str>,
    now: NaiveDateTime,
) -> Result<bool, WindowExpiryError> {
    let Some(text) = reset_at else {
        return Ok(false);
    };
    let stored = parse_stored_reset_at(text)?;
    Ok(stored <= now)
}

// A stored timestamp is isoformat's output or a naive value; the offset is dropped, not converted, because LiteLLM strips tzinfo before comparing.
pub(crate) fn parse_stored_reset_at(text: &str) -> Result<NaiveDateTime, WindowExpiryError> {
    if let Ok(aware) = DateTime::parse_from_rfc3339(text) {
        return Ok(aware.naive_local());
    }
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(naive);
        }
    }
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_source| {
        WindowExpiryError::Unparseable {
            text: text.to_owned(),
        }
    })?;
    date.and_hms_opt(0, 0, 0)
        .ok_or_else(|| WindowExpiryError::Unparseable {
            text: text.to_owned(),
        })
}

mod tests;
