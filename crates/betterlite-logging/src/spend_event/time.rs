use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::spend_bodies::session::SpendTimestamp;

const NAIVE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";
const SPACED_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";

#[derive(Debug, Error)]
pub enum EventTimeError {
    #[error("{text} is not an ISO 8601 timestamp: {source}")]
    Invalid {
        text: String,
        #[source]
        source: chrono::ParseError,
    },
}

// The wire text of a Python datetime, aware or naive, always written the way pydantic writes it
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct EventTime(String);

fn render(moment: &NaiveDateTime) -> String {
    let base = moment.format("%Y-%m-%dT%H:%M:%S");
    match moment.and_utc().timestamp_subsec_micros() {
        0 => base.to_string(),
        micros => format!("{base}.{micros:06}"),
    }
}

impl EventTime {
    #[must_use]
    pub fn new(moment: SpendTimestamp) -> Self {
        Self(match moment {
            SpendTimestamp::Aware(aware) if aware.offset().local_minus_utc() == 0_i32 => {
                format!("{}Z", render(&aware.naive_local()))
            }
            SpendTimestamp::Aware(aware) => {
                format!("{}{}", render(&aware.naive_local()), aware.format("%:z"))
            }
            SpendTimestamp::Naive(naive) => render(&naive),
        })
    }
}

impl TryFrom<String> for EventTime {
    type Error = EventTimeError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if let Ok(moment) = DateTime::parse_from_rfc3339(&text) {
            return Ok(Self::new(SpendTimestamp::Aware(moment)));
        }
        match NaiveDateTime::parse_from_str(&text, NAIVE_FORMAT)
            .or_else(|_| NaiveDateTime::parse_from_str(&text, SPACED_FORMAT))
        {
            Ok(moment) => Ok(Self::new(SpendTimestamp::Naive(moment))),
            Err(source) => Err(EventTimeError::Invalid { text, source }),
        }
    }
}
