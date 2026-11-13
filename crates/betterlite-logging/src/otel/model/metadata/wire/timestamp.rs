use crate::otel::model::utils::time::{FixedLocalTimeResolver, to_seconds};
use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "RawTimestamp")]
pub struct Timestamp(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RawTimestamp {
    Seconds(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal),
    Formatted(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TimestampError {
    #[error("invalid timestamp")]
    Invalid,
}

impl TryFrom<RawTimestamp> for Timestamp {
    type Error = TimestampError;

    fn try_from(raw: RawTimestamp) -> Result<Self, Self::Error> {
        match raw {
            RawTimestamp::Seconds(seconds) => Ok(Self(seconds)),
            RawTimestamp::Formatted(text) => {
                let offset = chrono::FixedOffset::east_opt(0).ok_or(TimestampError::Invalid)?;
                let resolver = FixedLocalTimeResolver { offset };
                to_seconds(None, None, Some(&text), &resolver)
                    .map(Self)
                    .ok_or(TimestampError::Invalid)
            }
        }
    }
}
