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

// A Python datetime is aware or naive, and the wire text keeps whichever it was
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventTime(pub SpendTimestamp);

fn render(moment: &NaiveDateTime) -> String {
    let base = moment.format("%Y-%m-%dT%H:%M:%S");
    match moment.and_utc().timestamp_subsec_micros() {
        0 => base.to_string(),
        micros => format!("{base}.{micros:06}"),
    }
}

impl From<EventTime> for String {
    fn from(time: EventTime) -> Self {
        match time.0 {
            SpendTimestamp::Aware(moment) if moment.offset().local_minus_utc() == 0_i32 => {
                format!("{}Z", render(&moment.naive_local()))
            }
            SpendTimestamp::Aware(moment) => {
                format!("{}{}", render(&moment.naive_local()), moment.format("%:z"))
            }
            SpendTimestamp::Naive(moment) => render(&moment),
        }
    }
}

impl TryFrom<String> for EventTime {
    type Error = EventTimeError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if let Ok(moment) = DateTime::parse_from_rfc3339(&text) {
            return Ok(Self(SpendTimestamp::Aware(moment)));
        }
        match NaiveDateTime::parse_from_str(&text, NAIVE_FORMAT)
            .or_else(|_| NaiveDateTime::parse_from_str(&text, SPACED_FORMAT))
        {
            Ok(moment) => Ok(Self(SpendTimestamp::Naive(moment))),
            Err(source) => Err(EventTimeError::Invalid { text, source }),
        }
    }
}
