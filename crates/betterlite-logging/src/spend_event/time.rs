use chrono::{DateTime, NaiveDateTime};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::spend_bodies::session::SpendTimestamp;

const NAIVE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";
const SPACED_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";

fn render(moment: &NaiveDateTime) -> String {
    let base = moment.format("%Y-%m-%dT%H:%M:%S");
    match moment.and_utc().timestamp_subsec_micros() {
        0 => base.to_string(),
        micros => format!("{base}.{micros:06}"),
    }
}

impl Serialize for SpendTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = match self {
            Self::Aware(moment) if moment.offset().local_minus_utc() == 0 => {
                format!("{}Z", render(&moment.naive_local()))
            }
            Self::Aware(moment) => {
                format!("{}{}", render(&moment.naive_local()), moment.format("%:z"))
            }
            Self::Naive(moment) => render(moment),
        };
        serializer.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for SpendTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if let Ok(moment) = DateTime::parse_from_rfc3339(&text) {
            return Ok(Self::Aware(moment));
        }
        NaiveDateTime::parse_from_str(&text, NAIVE_FORMAT)
            .or_else(|_| NaiveDateTime::parse_from_str(&text, SPACED_FORMAT))
            .map(Self::Naive)
            .map_err(D::Error::custom)
    }
}
