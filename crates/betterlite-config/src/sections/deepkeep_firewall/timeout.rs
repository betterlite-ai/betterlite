use std::num::ParseFloatError;
use std::time::{Duration, TryFromFloatSecsError};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(f64),
    Text(String),
    Boolean(bool),
    Empty(()),
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "Seconds")]
pub(super) struct Timeout(pub(super) Duration);

#[derive(Debug, Error)]
pub(super) enum Error {
    #[error("timeout must be numeric")]
    Number {
        #[source]
        source: ParseFloatError,
    },
    #[error("timeout must be a finite nonnegative duration")]
    Duration {
        #[source]
        source: TryFromFloatSecsError,
    },
}

impl Default for Timeout {
    fn default() -> Self {
        Self(Duration::from_secs(600))
    }
}

impl TryFrom<Seconds> for Timeout {
    type Error = Error;

    fn try_from(value: Seconds) -> Result<Self, Self::Error> {
        let seconds = match value {
            Seconds::Empty(()) => return Ok(Self::default()),
            Seconds::Text(text) if text.is_empty() => return Ok(Self::default()),
            Seconds::Text(text) => text
                .trim()
                .parse::<f64>()
                .map_err(|source| Error::Number { source })?,
            Seconds::Number(number) => number,
            Seconds::Boolean(boolean) => f64::from(u8::from(boolean)),
        };
        let duration =
            Duration::try_from_secs_f64(seconds).map_err(|source| Error::Duration { source })?;
        Ok(Self(duration))
    }
}
